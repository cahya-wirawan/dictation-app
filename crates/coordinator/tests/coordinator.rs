//! Coordinator actor end to end: fake microphone + fake or real worker.
//! M1 exit criteria: deterministic flows and no idle capture.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dictation_audio::capture::{CaptureError, NativeFormat};
use dictation_audio::detector::DetectorConfig;
use dictation_audio::dsp::AudioSource;
use dictation_audio::endpoint::EndpointConfig;
use dictation_coordinator::actor::*;
use dictation_coordinator::adapters::*;
use dictation_domain::Phase;
use dictation_domain::session::{AppError, DraftReason, JobKey, Outcome};
use dictation_domain::{SessionId, WorkerEpoch};
use transcription_protocol::Language;

const T: Duration = Duration::from_secs(30);
const BASE_SHA256: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn jfk() -> Vec<f32> {
    hound::WavReader::open(repo().join("vendor/whisper.cpp/samples/jfk.wav"))
        .unwrap()
        .samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect()
}

// ---------- fakes ----------

#[derive(Clone, Default)]
struct MicProbe {
    open: Arc<AtomicUsize>,
    opened_total: Arc<AtomicUsize>,
}

struct FakeMic {
    data: Arc<Vec<f32>>,
    pos: usize,
    chunk: usize,
    fail_at: Option<(usize, CaptureError)>,
    drains: usize,
    probe: MicProbe,
    closed: bool,
}

impl AudioSource for FakeMic {
    fn format(&self) -> NativeFormat {
        NativeFormat {
            sample_rate: 16_000,
            channels: 1,
            sample_format: "f32",
        }
    }
    fn drain_into(&mut self, out: &mut Vec<f32>) -> Result<usize, CaptureError> {
        self.drains += 1;
        if let Some((n, e)) = self.fail_at
            && self.drains >= n
        {
            return Err(e);
        }
        let end = (self.pos + self.chunk).min(self.data.len());
        out.extend_from_slice(&self.data[self.pos..end]);
        let n = end - self.pos;
        self.pos = end;
        Ok(n)
    }
    fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            self.probe.open.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn mic_factory(
    data: Vec<f32>,
    chunk: usize,
    fail_at: Option<(usize, CaptureError)>,
    probe: MicProbe,
) -> SourceFactory {
    let data = Arc::new(data);
    Box::new(move |_device| {
        probe.open.fetch_add(1, Ordering::SeqCst);
        probe.opened_total.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeMic {
            data: Arc::clone(&data),
            pos: 0,
            chunk,
            fail_at,
            drains: 0,
            probe: probe.clone(),
            closed: false,
        }) as Box<dyn AudioSource>)
    })
}

/// Returns fixed text after `delay`; records jobs; kill bumps the epoch.
struct FakeWorker {
    text: String,
    delay: Duration,
    epoch: WorkerEpoch,
    jobs: Arc<Mutex<Vec<JobKey>>>,
    kills: Arc<AtomicUsize>,
}

impl TranscriberAdapter for FakeWorker {
    fn start(&mut self, job: JobKey, pcm: Vec<f32>, events: Sender<Msg>) {
        assert!(!pcm.is_empty());
        self.jobs.lock().unwrap().push(job);
        let (text, delay) = (self.text.clone(), self.delay);
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let _ = events.send(Msg::Adapter(AdapterEvent::TranscriptionDone { job, text }));
        });
    }
    fn kill(&mut self) -> WorkerEpoch {
        self.kills.fetch_add(1, Ordering::SeqCst);
        self.epoch += 1;
        self.epoch
    }
}

#[derive(Default)]
struct Recorded {
    phases: Vec<Phase>,
    outcomes: Vec<Outcome>,
    levels: usize,
}

struct RecNotifier(Arc<Mutex<Recorded>>, Sender<Outcome>);

impl Notifier for RecNotifier {
    fn phase(&mut self, p: Phase) {
        self.0.lock().unwrap().phases.push(p);
    }
    fn outcome(&mut self, o: Outcome) {
        self.0.lock().unwrap().outcomes.push(o);
        let _ = self.1.send(o);
    }
    fn level(&mut self, _: f32) {
        self.0.lock().unwrap().levels += 1;
    }
}

struct Rig {
    coord: Coordinator,
    h: CoordinatorHandle,
    outcomes: Receiver<Outcome>,
    rec: Arc<Mutex<Recorded>>,
    mic: MicProbe,
}

impl Rig {
    fn new(
        factory_data: Vec<f32>,
        chunk: usize,
        fail_at: Option<(usize, CaptureError)>,
        endpoint: EndpointConfig,
        worker: Box<dyn TranscriberAdapter>,
    ) -> Self {
        let mic = MicProbe::default();
        let rec = Arc::new(Mutex::new(Recorded::default()));
        let (otx, outcomes) = mpsc::channel();
        let coord = Coordinator::spawn(
            Adapters {
                capture: Box::new(DspCapture::new(
                    mic_factory(factory_data, chunk, fail_at, mic.clone()),
                    Arc::new(Mutex::new(CaptureSettings {
                        device_id: None,
                        endpoint,
                    })),
                    DetectorConfig::default(),
                    Duration::from_millis(2),
                )),
                transcriber: worker,
                delivery: Box::new(CopyOnlyDelivery),
                notifier: Box::new(RecNotifier(Arc::clone(&rec), otx)),
            },
            0,
        );
        let h = coord.handle();
        Self {
            coord,
            h,
            outcomes,
            rec,
            mic,
        }
    }

    fn next_outcome(&self) -> Outcome {
        self.outcomes.recv_timeout(T).expect("no outcome")
    }

    fn snapshot(&self) -> Snapshot {
        self.h.snapshot(T).unwrap()
    }

    /// No idle capture: the microphone is closed whenever we are not recording.
    fn assert_mic_closed(&self) {
        let s = self.snapshot();
        assert!(
            !s.capture_open,
            "coordinator thinks capture is open in {:?}",
            s.phase
        );
        assert_eq!(
            self.mic.open.load(Ordering::SeqCst),
            0,
            "fake microphone still open"
        );
    }
}

fn fake_worker(
    text: &str,
    delay: Duration,
) -> (Box<FakeWorker>, Arc<Mutex<Vec<JobKey>>>, Arc<AtomicUsize>) {
    let jobs = Arc::new(Mutex::new(vec![]));
    let kills = Arc::new(AtomicUsize::new(0));
    (
        Box::new(FakeWorker {
            text: text.into(),
            delay,
            epoch: 0,
            jobs: Arc::clone(&jobs),
            kills: Arc::clone(&kills),
        }),
        jobs,
        kills,
    )
}

fn speech_then_silence() -> Vec<f32> {
    // First phrase of jfk.wav ("And so my fellow Americans") + 2 s silence.
    let mut v: Vec<f32> = jfk().into_iter().take(16_000 * 2 + 4_000).collect();
    v.extend(vec![0.0; 16_000 * 2]);
    v
}

// ---------- tests with the fake worker ----------

#[test]
fn toggle_speech_to_draft_with_mic_closed() {
    let (w, jobs, _) = fake_worker("Und so, meine Mitbürger", Duration::from_millis(20));
    let r = Rig::new(
        speech_then_silence(),
        800,
        None,
        EndpointConfig::default(),
        w,
    );
    assert!(!r.mic.open.load(Ordering::SeqCst) > 0);
    r.assert_mic_closed(); // nothing open before the first press
    r.h.send(Command::Toggle);
    assert_eq!(
        r.next_outcome(),
        Outcome::DraftReady(DraftReason::ReviewMode)
    );
    r.assert_mic_closed();
    assert_eq!(jobs.lock().unwrap().len(), 1);
    let draft = r.h.draft(T).unwrap();
    assert_eq!(draft.text, "Und so, meine Mitbürger");
    let phases = r.rec.lock().unwrap().phases.clone();
    assert_eq!(
        phases,
        [
            Phase::Starting,
            Phase::Listening,
            Phase::Transcribing,
            Phase::DraftReady
        ]
    );
    assert!(r.rec.lock().unwrap().levels > 0);

    // A second press with a pending draft is refused, not discarded.
    r.h.send(Command::Toggle);
    assert_eq!(r.next_outcome(), Outcome::Rejected(AppError::DraftPending));
    r.assert_mic_closed();
    assert_eq!(r.mic.opened_total.load(Ordering::SeqCst), 1);

    r.h.send(Command::DraftDiscard { id: draft.id });
    assert_eq!(r.next_outcome(), Outcome::DraftDiscarded);
    assert_eq!(r.snapshot().phase, Phase::Idle);
    r.coord.shutdown();
}

#[test]
fn silence_is_no_speech_without_inference() {
    let (w, jobs, _) = fake_worker("x", Duration::ZERO);
    let r = Rig::new(
        vec![0.0; 16_000 * 10],
        1600,
        None,
        EndpointConfig {
            no_speech_timeout_ms: 3000,
            ..Default::default()
        },
        w,
    );
    r.h.send(Command::Toggle);
    assert_eq!(r.next_outcome(), Outcome::NoSpeech);
    assert!(jobs.lock().unwrap().is_empty());
    r.assert_mic_closed();
    assert_eq!(r.snapshot().phase, Phase::Idle);
}

#[test]
fn second_toggle_is_explicit_stop_with_one_job() {
    let (w, jobs, _) = fake_worker("ok", Duration::ZERO);
    // Speech without trailing silence, delivered slowly: only an explicit stop finishes it.
    let speech: Vec<f32> = jfk().into_iter().skip(5_000).take(16_000 * 2).collect();
    let r = Rig::new(speech, 32, None, EndpointConfig::default(), w);
    r.h.send(Command::Toggle);
    std::thread::sleep(Duration::from_millis(1500));
    r.h.send(Command::Toggle);
    r.h.send(Command::Toggle); // races the first stop; must not start a new session
    let first = r.next_outcome();
    assert!(
        matches!(
            first,
            Outcome::DraftReady(_) | Outcome::Rejected(AppError::Busy)
        ),
        "{first:?}"
    );
    let o = if first == Outcome::Rejected(AppError::Busy) {
        r.next_outcome()
    } else {
        first
    };
    assert_eq!(o, Outcome::DraftReady(DraftReason::ReviewMode));
    assert_eq!(jobs.lock().unwrap().len(), 1);
    r.assert_mic_closed();
}

#[test]
fn cancel_while_listening_closes_mic_and_recovers() {
    let (w, jobs, _) = fake_worker("x", Duration::ZERO);
    let r = Rig::new(jfk(), 16, None, EndpointConfig::default(), w);
    r.h.send(Command::Toggle);
    std::thread::sleep(Duration::from_millis(200));
    assert!(r.snapshot().capture_open);
    r.h.send(Command::Cancel);
    assert_eq!(r.next_outcome(), Outcome::Canceled);
    assert_eq!(r.snapshot().phase, Phase::Idle);
    r.assert_mic_closed();
    assert!(jobs.lock().unwrap().is_empty());
}

#[test]
fn cancel_while_transcribing_kills_worker_and_drops_late_result() {
    let (w, jobs, kills) = fake_worker("late text", Duration::from_millis(600));
    let r = Rig::new(
        speech_then_silence(),
        1600,
        None,
        EndpointConfig::default(),
        w,
    );
    r.h.send(Command::Toggle);
    let deadline = Instant::now() + T;
    while r.snapshot().phase != Phase::Transcribing {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    r.h.send(Command::Cancel);
    assert_eq!(r.next_outcome(), Outcome::Canceled);
    assert_eq!(kills.load(Ordering::SeqCst), 1);
    assert_eq!(r.snapshot().worker_epoch, 1);
    std::thread::sleep(Duration::from_millis(800)); // late result arrives and is ignored
    assert_eq!(r.snapshot().phase, Phase::Idle);
    assert!(r.h.draft(T).is_none());
    assert!(
        r.outcomes.try_recv().is_err(),
        "late result must not produce an outcome"
    );
    // The next session uses the new epoch.
    r.h.send(Command::Toggle);
    assert_eq!(
        r.next_outcome(),
        Outcome::DraftReady(DraftReason::ReviewMode)
    );
    assert_eq!(jobs.lock().unwrap().last().unwrap().worker_epoch, 1);
    r.assert_mic_closed();
}

#[test]
fn microphone_fault_aborts_without_inference() {
    for (err, app) in [
        (CaptureError::Disconnected, AppError::MicDisconnected),
        (CaptureError::Overflow, AppError::AudioOverflow),
    ] {
        let (w, jobs, _) = fake_worker("x", Duration::ZERO);
        let r = Rig::new(jfk(), 1600, Some((10, err)), EndpointConfig::default(), w);
        r.h.send(Command::Toggle);
        assert_eq!(r.next_outcome(), Outcome::Failed(app));
        assert!(jobs.lock().unwrap().is_empty());
        r.assert_mic_closed();
        assert_eq!(r.snapshot().phase, Phase::Idle);
    }
}

#[test]
fn not_ready_rejects_without_opening_mic() {
    let (w, _, _) = fake_worker("x", Duration::ZERO);
    let r = Rig::new(jfk(), 1600, None, EndpointConfig::default(), w);
    r.h.send(Command::SetReady(Err(AppError::ModelUnavailable)));
    r.h.send(Command::Toggle);
    assert_eq!(
        r.next_outcome(),
        Outcome::Rejected(AppError::ModelUnavailable)
    );
    assert_eq!(r.mic.opened_total.load(Ordering::SeqCst), 0);
}

#[test]
fn shutdown_while_listening_closes_mic() {
    let (w, _, _) = fake_worker("x", Duration::ZERO);
    let r = Rig::new(jfk(), 16, None, EndpointConfig::default(), w);
    r.h.send(Command::Toggle);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(r.mic.open.load(Ordering::SeqCst), 1);
    let mic = r.mic.clone();
    r.coord.shutdown();
    let deadline = Instant::now() + Duration::from_secs(5);
    while mic.open.load(Ordering::SeqCst) != 0 {
        assert!(
            Instant::now() < deadline,
            "microphone left open after shutdown"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn many_sessions_never_leave_capture_open() {
    let (w, _, _) = fake_worker("t", Duration::from_millis(1));
    let r = Rig::new(
        speech_then_silence(),
        1600,
        None,
        EndpointConfig::default(),
        w,
    );
    for i in 0..15 {
        r.h.send(Command::Toggle);
        if i % 3 == 0 {
            std::thread::sleep(Duration::from_millis(30));
            r.h.send(Command::Cancel);
            let o = r.next_outcome();
            assert!(
                matches!(o, Outcome::Canceled | Outcome::DraftReady(_)),
                "{o:?}"
            );
        } else {
            assert_eq!(
                r.next_outcome(),
                Outcome::DraftReady(DraftReason::ReviewMode)
            );
        }
        if let Some(d) = r.snapshot().draft {
            r.h.send(Command::DraftDiscard { id: d.id });
            assert_eq!(r.next_outcome(), Outcome::DraftDiscarded);
        }
        r.assert_mic_closed();
    }
    assert_eq!(r.mic.opened_total.load(Ordering::SeqCst), 15);
}

// ---------- real worker ----------

fn real_worker_cfg() -> Option<WorkerConfig> {
    let exe = std::env::current_exe()
        .ok()?
        .parent()?
        .parent()?
        .join(if cfg!(windows) {
            "dictation-worker.exe"
        } else {
            "dictation-worker"
        });
    let model = repo().join(".models/ggml-base.bin");
    if !exe.is_file() || !model.is_file() {
        eprintln!(
            "SKIP: needs {} (cargo build -p transcription-worker) and {}",
            exe.display(),
            model.display()
        );
        return None;
    }
    Some(WorkerConfig {
        exe,
        model,
        model_sha256: BASE_SHA256.into(),
        language: Arc::new(Mutex::new(Language::try_from("en".to_string()).unwrap())),
        job_timeout: Duration::from_secs(60),
    })
}

fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn real_worker_end_to_end_and_cancel_respawn() {
    let Some(cfg) = real_worker_cfg() else { return };
    let mut audio = jfk();
    audio.extend(vec![0.0; 16_000 * 2]);
    // 1500 ms silence keeps jfk.wav's ~1.03 s oratorical pauses inside one phrase.
    let r = Rig::new(
        audio,
        4_000,
        None,
        EndpointConfig {
            silence_ms: 1500,
            ..Default::default()
        },
        Box::new(WorkerTranscriber::new(cfg, 0)),
    );
    r.h.send(Command::Toggle);
    assert_eq!(
        r.next_outcome(),
        Outcome::DraftReady(DraftReason::ReviewMode)
    );
    let d = r.h.draft(T).unwrap();
    let text = normalize(&d.text);
    assert!(
        text.contains("ask not what your country can do for you"),
        "{text}"
    );
    r.assert_mic_closed();
    r.h.send(Command::DraftDiscard { id: d.id });
    assert_eq!(r.next_outcome(), Outcome::DraftDiscarded);

    // Cancel during real inference, then a new session on the respawned worker.
    r.h.send(Command::Toggle);
    let deadline = Instant::now() + T;
    while r.snapshot().phase != Phase::Transcribing {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    r.h.send(Command::Cancel);
    assert_eq!(r.next_outcome(), Outcome::Canceled);
    assert_eq!(r.snapshot().worker_epoch, 1);
    r.h.send(Command::Toggle);
    assert_eq!(
        r.next_outcome(),
        Outcome::DraftReady(DraftReason::ReviewMode)
    );
    assert!(normalize(&r.h.draft(T).unwrap().text).contains("my fellow americans"));
    r.assert_mic_closed();
    let _ = SessionId::random(); // keep import used on all cfgs
    let _ = AtomicBool::new(false);
}
