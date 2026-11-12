//! Soak (docs/08 §6): 500 mixed sessions through the real actor, DSP capture
//! and reducer with a fake microphone and worker. Mix per session: endpoint
//! finish, explicit stop, cancel while listening, cancel while transcribing
//! (worker kill + epoch bump), silence (no speech) and a device fault
//! (disconnect mid-capture). Asserts after every session that the mic is
//! closed and exactly one terminal outcome arrived; at the end, that no job
//! ran twice and that resident memory has no monotonic trend (Linux).
//!
//! Also checks the diagnostics export: a distinctive transcript token is
//! never present in the report (docs/08 §2 "Logging").

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dictation_audio::capture::{CaptureError, NativeFormat};
use dictation_audio::detector::DetectorConfig;
use dictation_audio::dsp::AudioSource;
use dictation_audio::endpoint::EndpointConfig;
use dictation_coordinator::actor::*;
use dictation_coordinator::adapters::*;
use dictation_coordinator::diagnostics::{
    Diagnostics, RecordingNotifier, ReportFacts, SettingsSummary,
};
use dictation_domain::Phase;
use dictation_domain::WorkerEpoch;
use dictation_domain::session::{AppError, DraftReason, JobKey, Outcome};

const T: Duration = Duration::from_secs(30);
const SECRET: &str = "ZEBRA-PASSPHRASE-7731";
const SESSIONS: usize = 500;

fn jfk_head() -> Vec<f32> {
    let p: PathBuf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/whisper.cpp/samples/jfk.wav");
    let mut v: Vec<f32> = hound::WavReader::open(p)
        .unwrap()
        .samples::<i16>()
        .take(16_000 * 2 + 4_000)
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect();
    v.extend(vec![0.0; 16_000 * 2]);
    v
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Endpoint,
    Stop,
    CancelListening,
    CancelTranscribing,
    Silence,
    DeviceFault,
}

const MIX: [Kind; 6] = [
    Kind::Endpoint,
    Kind::Stop,
    Kind::CancelListening,
    Kind::CancelTranscribing,
    Kind::Silence,
    Kind::DeviceFault,
];

struct Mic {
    data: Arc<Vec<f32>>,
    pos: usize,
    fail_after: Option<usize>,
    drains: usize,
    open: Arc<AtomicUsize>,
    closed: bool,
}

impl AudioSource for Mic {
    fn format(&self) -> NativeFormat {
        NativeFormat {
            sample_rate: 48_000,
            channels: 2,
            sample_format: "f32",
        }
    }
    fn drain_into(&mut self, out: &mut Vec<f32>) -> Result<usize, CaptureError> {
        self.drains += 1;
        if self.fail_after.is_some_and(|n| self.drains > n) {
            return Err(CaptureError::Disconnected);
        }
        // 16 kHz mono fixture presented as 48 kHz stereo (resampler in the path).
        let end = (self.pos + 533).min(self.data.len());
        for &s in &self.data[self.pos..end] {
            for _ in 0..3 {
                out.push(s);
                out.push(s);
            }
        }
        let n = (end - self.pos) * 6;
        self.pos = end;
        Ok(n)
    }
    fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            self.open.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

struct Worker {
    epoch: WorkerEpoch,
    delay: Duration,
    jobs: Arc<Mutex<Vec<JobKey>>>,
    kills: Arc<AtomicUsize>,
}

impl TranscriberAdapter for Worker {
    fn start(&mut self, job: JobKey, _pcm: Vec<f32>, events: Sender<Msg>) {
        self.jobs.lock().unwrap().push(job);
        let delay = self.delay;
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let _ = events.send(Msg::Adapter(AdapterEvent::TranscriptionDone {
                job,
                text: SECRET.into(),
            }));
        });
    }
    fn kill(&mut self) -> WorkerEpoch {
        self.kills.fetch_add(1, Ordering::SeqCst);
        self.epoch += 1;
        self.epoch
    }
    fn switch_model(&mut self, _m: ModelRef, _e: Sender<Msg>) {}
}

struct Chan(Sender<Outcome>, Sender<Phase>);
impl Notifier for Chan {
    fn phase(&mut self, p: Phase) {
        let _ = self.1.send(p);
    }
    fn outcome(&mut self, o: Outcome) {
        let _ = self.0.send(o);
    }
    fn level(&mut self, _: f32) {}
}

#[cfg(target_os = "linux")]
fn rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines()
        .find(|l| l.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
#[cfg(not(target_os = "linux"))]
fn rss_kib() -> Option<u64> {
    None
}

#[test]
fn soak_500_mixed_sessions() {
    let data = Arc::new(jfk_head());
    let silence = Arc::new(vec![0.0f32; 16_000 * 4]);
    let open = Arc::new(AtomicUsize::new(0));
    let opened = Arc::new(AtomicUsize::new(0));
    let plan: Arc<Mutex<Kind>> = Arc::new(Mutex::new(Kind::Endpoint));
    let factory: SourceFactory = {
        let (open, opened, plan) = (Arc::clone(&open), Arc::clone(&opened), Arc::clone(&plan));
        Box::new(move |_dev| {
            open.fetch_add(1, Ordering::SeqCst);
            opened.fetch_add(1, Ordering::SeqCst);
            let kind = *plan.lock().unwrap();
            let (d, fail_after) = match kind {
                Kind::Silence => (Arc::clone(&silence), None),
                Kind::DeviceFault => (Arc::clone(&data), Some(20)),
                _ => (Arc::clone(&data), None),
            };
            Ok(Box::new(Mic {
                data: d,
                pos: 0,
                fail_after,
                drains: 0,
                open: Arc::clone(&open),
                closed: false,
            }) as Box<dyn AudioSource>)
        })
    };
    let jobs = Arc::new(Mutex::new(vec![]));
    let kills = Arc::new(AtomicUsize::new(0));
    let diagnostics = Diagnostics::new();
    let (otx, outcomes) = mpsc::channel();
    let (ptx, phases) = mpsc::channel();
    let coord = Coordinator::spawn(
        Adapters {
            capture: Box::new(DspCapture::new(
                factory,
                Arc::new(Mutex::new(CaptureSettings {
                    device_id: None,
                    endpoint: EndpointConfig {
                        no_speech_timeout_ms: 1500,
                        ..EndpointConfig::default()
                    },
                })),
                DetectorConfig::default(),
                Duration::from_millis(1),
            )),
            transcriber: Box::new(Worker {
                epoch: 0,
                delay: Duration::from_millis(15),
                jobs: Arc::clone(&jobs),
                kills: Arc::clone(&kills),
            }),
            delivery: Box::new(CopyOnlyDelivery),
            notifier: Box::new(RecordingNotifier {
                inner: Chan(otx, ptx),
                diagnostics: Arc::clone(&diagnostics),
            }),
        },
        0,
    );
    let h = coord.handle();
    let wait_phase = |want: Phase| loop {
        let p = phases.recv_timeout(T).expect("phase");
        if p == want {
            break;
        }
    };

    let mut rss = vec![];
    for i in 0..SESSIONS {
        let kind = MIX[i % MIX.len()];
        *plan.lock().unwrap() = kind;
        while phases.try_recv().is_ok() {}
        h.send(Command::Toggle);
        match kind {
            Kind::Stop => {
                wait_phase(Phase::Listening);
                std::thread::sleep(Duration::from_millis(5));
                h.send(Command::Stop);
            }
            Kind::CancelListening => {
                wait_phase(Phase::Listening);
                h.send(Command::Cancel);
            }
            Kind::CancelTranscribing => {
                wait_phase(Phase::Transcribing);
                h.send(Command::Cancel);
            }
            _ => {}
        }
        let o = outcomes
            .recv_timeout(T)
            .unwrap_or_else(|_| panic!("session {i} {kind:?}: no outcome"));
        let ok = match kind {
            Kind::Endpoint => o == Outcome::DraftReady(DraftReason::ReviewMode),
            // A stop very early in the phrase may legitimately find no speech.
            Kind::Stop => matches!(o, Outcome::DraftReady(_) | Outcome::NoSpeech),
            Kind::CancelListening | Kind::CancelTranscribing => o == Outcome::Canceled,
            Kind::Silence => o == Outcome::NoSpeech,
            Kind::DeviceFault => o == Outcome::Failed(AppError::MicDisconnected),
        };
        assert!(ok, "session {i} {kind:?}: {o:?}");
        let snap = h.snapshot(T).unwrap();
        if let Some(d) = snap.draft {
            h.send(Command::DraftDiscard { id: d.id });
            assert_eq!(outcomes.recv_timeout(T).unwrap(), Outcome::DraftDiscarded);
        }
        // Exactly one terminal outcome per session.
        std::thread::sleep(Duration::from_millis(2));
        assert!(outcomes.try_recv().is_err(), "session {i}: extra outcome");
        assert_eq!(open.load(Ordering::SeqCst), 0, "session {i}: mic left open");
        assert!(!h.snapshot(T).unwrap().capture_open);
        if i == 100 || i == SESSIONS - 1 {
            rss.push(rss_kib());
        }
    }
    assert_eq!(opened.load(Ordering::SeqCst), SESSIONS);
    let jobs = jobs.lock().unwrap();
    let unique: HashSet<_> = jobs
        .iter()
        .map(|j| (j.session, j.generation, j.worker_epoch))
        .collect();
    assert_eq!(unique.len(), jobs.len(), "a job ran twice");
    assert!(kills.load(Ordering::SeqCst) >= SESSIONS / MIX.len());
    eprintln!(
        "soak: {SESSIONS} sessions, {} jobs, {} worker kills, RSS KiB after warm-up/end: {rss:?}",
        jobs.len(),
        kills.load(Ordering::SeqCst)
    );
    if let [Some(a), Some(b)] = rss[..] {
        // >10% growth after warm-up needs investigation (docs/08 §6); 2 MiB slack
        // absorbs allocator noise in a small test process.
        assert!(b <= a + a / 10 + 2048, "RSS grew from {a} to {b} KiB");
    }

    // Diagnostics: codes only, bounded, and never the dictated token.
    let report = diagnostics.report(ReportFacts {
        app_version: "test".into(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        build_profile: "test",
        protocol_version: 1,
        active_model: None,
        installed_models: vec![],
        permissions: serde_json::json!({}),
        readiness: None,
        settings: SettingsSummary {
            schema_version: 2,
            delivery_mode: serde_json::json!("copy_only"),
            language: "auto".into(),
            silence_ms: 1000,
            max_capture_ms: 120_000,
            custom_input_device: false,
            excluded_app_count: 0,
            show_hud: true,
        },
    });
    let json = serde_json::to_string_pretty(&report).unwrap();
    assert!(!json.contains(SECRET), "transcript leaked into diagnostics");
    assert!(json.contains("MIC_DISCONNECTED"));
    assert!(report.recent_outcomes.len() <= dictation_coordinator::diagnostics::MAX_EVENTS);
    coord.shutdown();
}
