//! Production adapters for the coordinator.
//!
//! - [`DspCapture`]: opens an [`AudioSource`] (live CPAL or a test fake) and
//!   runs a [`DspSession`] that reports straight to the actor.
//! - [`WorkerTranscriber`]: one resident `dictation-worker`, hard-killed on
//!   cancel and respawned in the background with a new epoch.
//! - [`CopyOnlyDelivery`]: M1 delivery: never pastes; Copy writes the draft
//!   to the clipboard. Automatic delivery arrives with the M3 platform adapter.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use dictation_audio::capture::{self, CaptureError};
use dictation_audio::detector::{DetectorConfig, EarshotDetector};
use dictation_audio::dsp::{AudioSource, DspEvent, DspFault, DspSession, LiveCapture};
use dictation_audio::endpoint::{EndpointConfig, Outcome as EpOutcome};
use dictation_domain::session::{AppError, CaptureOutcome, DeliveryMode, JobKey};
use dictation_domain::{Generation, SessionId, WorkerEpoch};
use transcription_protocol::client::{CancelHandle, ClientError, WorkerClient};
use transcription_protocol::{JobId, Language, WorkerErrorCode};

use crate::actor::{AdapterEvent, CaptureAdapter, DeliveryAdapter, Msg, TranscriberAdapter};

/// Opens an audio source for a session on the given device (`None`: the
/// current system default). Called on the actor thread.
pub type SourceFactory =
    Box<dyn FnMut(Option<&str>) -> Result<Box<dyn AudioSource>, CaptureError> + Send>;

/// Capture settings read at every `open`, so changes apply to the next
/// capture without restarting anything (docs/04 §3 `update_settings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSettings {
    pub device_id: Option<String>,
    pub endpoint: EndpointConfig,
}

pub type SharedCaptureSettings = Arc<Mutex<CaptureSettings>>;

pub fn capture_error(e: CaptureError) -> AppError {
    match e {
        CaptureError::PermissionDenied => AppError::MicDenied,
        CaptureError::Overflow => AppError::AudioOverflow,
        CaptureError::NoDevice | CaptureError::DeviceNotAvailable | CaptureError::Disconnected => {
            AppError::MicDisconnected
        }
        CaptureError::UnsupportedFormat | CaptureError::Host | CaptureError::Thread => {
            AppError::MicDisconnected
        }
    }
}

/// Live microphone factory: opens the configured device (or the current
/// default, resolved to a concrete id; ADR-011).
pub fn live_source() -> SourceFactory {
    Box::new(move |device_id| {
        let (handle, reader) = capture::start(device_id)?;
        Ok(Box::new(LiveCapture { handle, reader }) as Box<dyn AudioSource>)
    })
}

pub struct DspCapture {
    factory: SourceFactory,
    settings: SharedCaptureSettings,
    detector: DetectorConfig,
    poll: Duration,
    sessions: HashMap<SessionId, DspSession>,
}

impl DspCapture {
    pub fn new(
        factory: SourceFactory,
        settings: SharedCaptureSettings,
        detector: DetectorConfig,
        poll: Duration,
    ) -> Self {
        Self {
            factory,
            settings,
            detector,
            poll,
            sessions: HashMap::new(),
        }
    }
}

impl CaptureAdapter for DspCapture {
    fn open(
        &mut self,
        id: SessionId,
        generation: Generation,
        events: Sender<Msg>,
    ) -> Result<(), AppError> {
        // Finished sessions have already closed their device; reap them.
        self.sessions.clear();
        let settings = self
            .settings
            .lock()
            .map(|s| s.clone())
            .map_err(|_| AppError::NotReady)?;
        let source = (self.factory)(settings.device_id.as_deref()).map_err(capture_error)?;
        let sink = move |ev: DspEvent| {
            let e = match ev {
                DspEvent::Level(level) => AdapterEvent::CaptureLevel {
                    id,
                    generation,
                    level,
                },
                DspEvent::Onset => AdapterEvent::CaptureOnset { id, generation },
                DspEvent::Finished(EpOutcome::Utterance { samples, .. }) => {
                    AdapterEvent::CaptureFinished {
                        id,
                        generation,
                        outcome: CaptureOutcome::Speech,
                        pcm: Some(samples),
                    }
                }
                DspEvent::Finished(EpOutcome::NoSpeech) => AdapterEvent::CaptureFinished {
                    id,
                    generation,
                    outcome: CaptureOutcome::NoSpeech,
                    pcm: None,
                },
                DspEvent::Finished(EpOutcome::TooShort) => AdapterEvent::CaptureFinished {
                    id,
                    generation,
                    outcome: CaptureOutcome::TooShort,
                    pcm: None,
                },
                DspEvent::Fault(DspFault::Capture(e)) => AdapterEvent::CaptureFailed {
                    id,
                    generation,
                    error: capture_error(e),
                },
                DspEvent::Fault(DspFault::InvalidAudio) => AdapterEvent::CaptureFailed {
                    id,
                    generation,
                    error: AppError::AudioOverflow,
                },
                DspEvent::Aborted => AdapterEvent::CaptureAborted { id, generation },
            };
            let _ = events.send(Msg::Adapter(e));
        };
        let session = DspSession::spawn(
            source,
            Box::new(EarshotDetector::new(self.detector)),
            settings.endpoint,
            self.poll,
            sink,
        )
        .map_err(|_| AppError::AudioOverflow)?;
        self.sessions.insert(id, session);
        Ok(())
    }

    fn finish(&mut self, id: SessionId) {
        if let Some(s) = self.sessions.get(&id) {
            s.stop();
        }
    }

    fn abort(&mut self, id: SessionId) {
        if let Some(s) = self.sessions.get(&id) {
            s.abort();
        }
    }
}

#[derive(Clone)]
pub struct WorkerConfig {
    pub exe: PathBuf,
    pub model: PathBuf,
    pub model_sha256: String,
    /// Read at every job start, so a language change applies to the next one.
    pub language: Arc<Mutex<Language>>,
    pub job_timeout: Duration,
}

/// Owns one resident worker. Jobs run on short-lived threads that hold the
/// client lock; `kill` uses a separate cancel handle so it never waits for
/// the job.
pub struct WorkerTranscriber {
    cfg: WorkerConfig,
    client: Arc<Mutex<Option<WorkerClient>>>,
    cancel: Arc<Mutex<Option<CancelHandle>>>,
    epoch: WorkerEpoch,
}

impl WorkerTranscriber {
    /// Spawns and warms the first worker in the background.
    pub fn new(cfg: WorkerConfig, epoch: WorkerEpoch) -> Self {
        let t = Self {
            cfg,
            client: Arc::new(Mutex::new(None)),
            cancel: Arc::new(Mutex::new(None)),
            epoch,
        };
        t.respawn_in_background(epoch);
        t
    }

    fn respawn_in_background(&self, epoch: WorkerEpoch) {
        let (cfg, client, cancel) = (
            self.cfg.clone(),
            Arc::clone(&self.client),
            Arc::clone(&self.cancel),
        );
        thread::spawn(move || {
            let Ok(mut slot) = client.lock() else { return };
            if slot.as_ref().is_some_and(|c| c.epoch() >= epoch) {
                return;
            }
            *slot = spawn_loaded(&cfg, epoch, &cancel).ok();
        });
    }
}

fn spawn_loaded(
    cfg: &WorkerConfig,
    epoch: WorkerEpoch,
    cancel: &Mutex<Option<CancelHandle>>,
) -> Result<WorkerClient, ClientError> {
    let mut c = WorkerClient::spawn(&cfg.exe, epoch, Duration::from_secs(10))?;
    if let Ok(mut h) = cancel.lock() {
        *h = Some(c.cancel_handle());
    }
    c.load_model(&cfg.model, &cfg.model_sha256, Duration::from_secs(60))?;
    Ok(c)
}

fn client_error(e: &ClientError) -> AppError {
    match e {
        ClientError::Timeout(_) => AppError::InferenceTimeout,
        ClientError::Worker {
            code: WorkerErrorCode::ModelInvalid | WorkerErrorCode::ModelLoadFailed,
            ..
        } => AppError::ModelInvalid,
        ClientError::Worker {
            code: WorkerErrorCode::ModelNotLoaded,
            ..
        } => AppError::ModelUnavailable,
        _ => AppError::WorkerCrash,
    }
}

impl TranscriberAdapter for WorkerTranscriber {
    fn start(&mut self, job: JobKey, pcm: Vec<f32>, events: Sender<Msg>) {
        let (cfg, client, cancel) = (
            self.cfg.clone(),
            Arc::clone(&self.client),
            Arc::clone(&self.cancel),
        );
        thread::spawn(move || {
            let send = |e| {
                let _ = events.send(Msg::Adapter(e));
            };
            let Ok(mut slot) = client.lock() else {
                return send(AdapterEvent::TranscriptionFailed {
                    job,
                    error: AppError::WorkerCrash,
                });
            };
            // Lazily (re)spawn if the background warm-up failed or lagged.
            if slot.as_ref().is_none_or(|c| c.epoch() != job.worker_epoch) {
                match spawn_loaded(&cfg, job.worker_epoch, &cancel) {
                    Ok(c) => *slot = Some(c),
                    Err(e) => {
                        return send(AdapterEvent::TranscriptionFailed {
                            job,
                            error: client_error(&e),
                        });
                    }
                }
            }
            let Some(c) = slot.as_mut() else { return };
            let id = JobId {
                session_id: job.session,
                generation: job.generation,
                worker_epoch: job.worker_epoch,
            };
            let language = cfg
                .language
                .lock()
                .map(|l| l.clone())
                .unwrap_or_else(|_| Language::auto());
            let result = c.transcribe(id, language, &pcm, cfg.job_timeout);
            drop(pcm); // no cached audio after the job
            match result {
                Ok(r) => send(AdapterEvent::TranscriptionDone { job, text: r.text }),
                Err(e) => {
                    if matches!(
                        e,
                        ClientError::Timeout(_) | ClientError::WorkerExited | ClientError::Frame(_)
                    ) {
                        *slot = None; // client already killed/reaped
                    }
                    send(AdapterEvent::TranscriptionFailed {
                        job,
                        error: client_error(&e),
                    });
                }
            }
        });
    }

    fn kill(&mut self) -> WorkerEpoch {
        if let Ok(h) = self.cancel.lock()
            && let Some(h) = h.as_ref()
        {
            h.kill();
        }
        self.epoch += 1;
        self.respawn_in_background(self.epoch);
        self.epoch
    }
}

/// M1 delivery: no automatic paste on any platform yet.
pub struct CopyOnlyDelivery;

impl DeliveryAdapter for CopyOnlyDelivery {
    fn capture_target(&mut self, _id: SessionId) -> bool {
        false
    }

    fn prepare(
        &mut self,
        id: SessionId,
        generation: Generation,
        _mode: DeliveryMode,
        _text: String,
        events: Sender<Msg>,
    ) {
        let _ = events.send(Msg::Adapter(AdapterEvent::DeliveryPrepared {
            id,
            generation,
            result: Err(AppError::PasteUnsupported),
        }));
    }

    fn dispatch(&mut self, _id: SessionId, _generation: Generation, _events: Sender<Msg>) {
        // Unreachable: prepare never succeeds, so the ledger never commits.
    }

    fn release(&mut self, _id: SessionId) {}

    fn copy(&mut self, id: SessionId, revision: u32, text: String, events: Sender<Msg>) {
        thread::spawn(move || {
            let ok = dictation_platform::clipboard_write_text(&text).is_ok();
            let _ = events.send(Msg::Adapter(AdapterEvent::CopyFinished {
                id,
                revision,
                ok,
            }));
        });
    }
}
