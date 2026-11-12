//! M3 delivery vertical slice (C1–C3) with a scripted platform: the real
//! actor, reducer and `PlatformDelivery`, a fake microphone and worker.
//! Exit criteria: one complete shortcut-to-text flow, focus-change and cancel
//! tests, and never more than one dispatch per transaction.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dictation_audio::capture::{CaptureError, NativeFormat};
use dictation_audio::detector::DetectorConfig;
use dictation_audio::dsp::AudioSource;
use dictation_audio::endpoint::EndpointConfig;
use dictation_coordinator::actor::*;
use dictation_coordinator::adapters::*;
use dictation_coordinator::delivery::*;
use dictation_domain::Phase;
use dictation_domain::WorkerEpoch;
use dictation_domain::session::{AppError, DraftReason, JobKey, Outcome};
use dictation_platform::{DispatchOutcome, FieldClass, TargetCheck};

const T: Duration = Duration::from_secs(20);

// ---------- scripted platform ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Target {
    app: u32,
    field: u32,
}

#[derive(Debug)]
struct World {
    focus: Target,
    field_class: FieldClass,
    app_id: Option<String>,
    capturable: bool,
    /// Modifiers stay held until this instant.
    modifiers_held_until: Option<Instant>,
    clipboard: Option<String>,
    clipboard_fails: bool,
    /// Simulate the user switching apps right after the clipboard write.
    switch_on_write: Option<Target>,
    send_result: DispatchOutcome,
    /// Every paste chord: (focused target, clipboard content at that time).
    pastes: Vec<(Target, Option<String>)>,
}

impl Default for World {
    fn default() -> Self {
        Self {
            focus: Target { app: 1, field: 1 },
            field_class: FieldClass::Editable,
            app_id: Some("com.apple.TextEdit".into()),
            capturable: true,
            modifiers_held_until: None,
            clipboard: None,
            clipboard_fails: false,
            switch_on_write: None,
            send_result: DispatchOutcome::Sent,
            pastes: vec![],
        }
    }
}

type W = Arc<Mutex<World>>;

struct FakePlatform(W);

impl TargetPlatform for FakePlatform {
    type Token = Target;
    fn capture(&mut self) -> Result<Target, AppError> {
        let w = self.0.lock().unwrap();
        if w.capturable {
            Ok(w.focus)
        } else {
            Err(AppError::PasteUnsupported)
        }
    }
    fn field(&self, _t: &Target) -> FieldClass {
        self.0.lock().unwrap().field_class
    }
    fn app_id(&self, _t: &Target) -> Option<String> {
        self.0.lock().unwrap().app_id.clone()
    }
    fn check(&mut self, t: &Target) -> TargetCheck {
        let w = self.0.lock().unwrap();
        if w.focus.app != t.app {
            TargetCheck::AppChanged
        } else if w.focus.field != t.field {
            TargetCheck::FieldChanged
        } else if w.field_class == FieldClass::Secure {
            TargetCheck::Secure
        } else {
            TargetCheck::Same
        }
    }
    fn modifiers_released(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let held = self
                .0
                .lock()
                .unwrap()
                .modifiers_held_until
                .is_some_and(|t| Instant::now() < t);
            if !held {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn write_clipboard(&mut self, text: &str) -> Result<(), AppError> {
        let mut w = self.0.lock().unwrap();
        if w.clipboard_fails {
            return Err(AppError::ClipboardBusy);
        }
        w.clipboard = Some(text.to_owned());
        if let Some(t) = w.switch_on_write.take() {
            w.focus = t;
        }
        Ok(())
    }
    fn send_paste(&mut self) -> Result<DispatchOutcome, AppError> {
        let mut w = self.0.lock().unwrap();
        let entry = (w.focus, w.clipboard.clone());
        w.pastes.push(entry);
        Ok(w.send_result)
    }
}

// ---------- fake mic / worker / notifier ----------

struct FakeMic {
    data: Arc<Vec<f32>>,
    pos: usize,
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
        let end = (self.pos + 1600).min(self.data.len());
        out.extend_from_slice(&self.data[self.pos..end]);
        let n = end - self.pos;
        self.pos = end;
        Ok(n)
    }
    fn close(&mut self) {}
}

struct FakeWorker {
    delay: Duration,
    epoch: WorkerEpoch,
    started: Arc<AtomicUsize>,
}

impl TranscriberAdapter for FakeWorker {
    fn start(&mut self, job: JobKey, _pcm: Vec<f32>, events: Sender<Msg>) {
        self.started.fetch_add(1, Ordering::SeqCst);
        let delay = self.delay;
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let _ = events.send(Msg::Adapter(AdapterEvent::TranscriptionDone {
                job,
                text: "Hallo, das ist ein Test".into(),
            }));
        });
    }
    fn kill(&mut self) -> WorkerEpoch {
        self.epoch += 1;
        self.epoch
    }
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

fn speech() -> Vec<f32> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/whisper.cpp/samples/jfk.wav");
    let mut v: Vec<f32> = hound::WavReader::open(p)
        .unwrap()
        .samples::<i16>()
        .take(16_000 * 2 + 4_000)
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect();
    v.extend(vec![0.0; 16_000 * 2]);
    v
}

struct Rig {
    _coord: Coordinator,
    h: CoordinatorHandle,
    world: W,
    outcomes: Receiver<Outcome>,
    phases: Receiver<Phase>,
    excluded: SharedExclusions,
}

impl Rig {
    fn new(world: World, worker_delay: Duration) -> Self {
        let world = Arc::new(Mutex::new(world));
        let data = Arc::new(speech());
        let (otx, outcomes) = mpsc::channel();
        let (ptx, phases) = mpsc::channel();
        let excluded: SharedExclusions = Arc::new(Mutex::new(vec![]));
        let w2 = Arc::clone(&world);
        let coord = Coordinator::spawn(
            Adapters {
                capture: Box::new(DspCapture::new(
                    Box::new(move |_| {
                        Ok(Box::new(FakeMic {
                            data: Arc::clone(&data),
                            pos: 0,
                        }) as Box<dyn AudioSource>)
                    }),
                    Arc::new(Mutex::new(CaptureSettings {
                        device_id: None,
                        endpoint: EndpointConfig::default(),
                    })),
                    DetectorConfig::default(),
                    Duration::from_millis(2),
                )),
                transcriber: Box::new(FakeWorker {
                    delay: worker_delay,
                    epoch: 0,
                    started: Arc::new(AtomicUsize::new(0)),
                }),
                delivery: Box::new(PlatformDelivery::spawn(
                    move || FakePlatform(w2),
                    Arc::clone(&excluded),
                )),
                notifier: Box::new(Chan(otx, ptx)),
            },
            0,
        );
        let h = coord.handle();
        Self {
            _coord: coord,
            h,
            world,
            outcomes,
            phases,
            excluded,
        }
    }

    fn outcome(&self) -> Outcome {
        self.outcomes.recv_timeout(T).expect("no outcome")
    }

    fn wait_phase(&self, want: Phase) {
        let deadline = Instant::now() + T;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.phases.recv_timeout(left) {
                Ok(p) if p == want => return,
                Ok(_) => {}
                Err(_) => panic!("never reached {want:?}"),
            }
        }
    }

    fn pastes(&self) -> Vec<(Target, Option<String>)> {
        self.world.lock().unwrap().pastes.clone()
    }
    fn clipboard(&self) -> Option<String> {
        self.world.lock().unwrap().clipboard.clone()
    }
}

const TEXT: &str = "Hallo, das ist ein Test";
const T1: Target = Target { app: 1, field: 1 };

#[test]
fn shortcut_to_text_inserts_exactly_once() {
    let r = Rig::new(World::default(), Duration::from_millis(10));
    r.h.send(Command::Toggle);
    assert_eq!(r.outcome(), Outcome::PasteSent);
    assert_eq!(
        r.pastes(),
        vec![(T1, Some(TEXT.into()))],
        "one paste into the original target with the text staged"
    );
    assert_eq!(r.h.snapshot(T).unwrap().phase, Phase::Idle);
    assert!(r.h.draft(T).is_none());
    // A second session also pastes once.
    r.h.send(Command::Toggle);
    assert_eq!(r.outcome(), Outcome::PasteSent);
    assert_eq!(r.pastes().len(), 2);
}

#[test]
fn app_switch_during_transcription_keeps_draft_without_touching_clipboard() {
    let r = Rig::new(World::default(), Duration::from_millis(300));
    r.h.send(Command::Toggle);
    r.wait_phase(Phase::Transcribing);
    r.world.lock().unwrap().focus = Target { app: 2, field: 1 };
    assert_eq!(
        r.outcome(),
        Outcome::DraftReady(DraftReason::Blocked(AppError::TargetChanged))
    );
    assert!(r.pastes().is_empty());
    assert_eq!(
        r.clipboard(),
        None,
        "blocked before commit: clipboard untouched"
    );
    assert_eq!(r.h.draft(T).unwrap().text, TEXT);
}

#[test]
fn field_switch_in_same_app_is_caught() {
    let r = Rig::new(World::default(), Duration::from_millis(300));
    r.h.send(Command::Toggle);
    r.wait_phase(Phase::Transcribing);
    r.world.lock().unwrap().focus = Target { app: 1, field: 2 };
    assert_eq!(
        r.outcome(),
        Outcome::DraftReady(DraftReason::Blocked(AppError::TargetChanged))
    );
    assert!(r.pastes().is_empty());
}

#[test]
fn secure_unknown_and_uncapturable_targets_are_copy_only() {
    for world in [
        World {
            field_class: FieldClass::Secure,
            ..World::default()
        },
        World {
            field_class: FieldClass::Unknown,
            ..World::default()
        },
        World {
            field_class: FieldClass::NotEditable,
            ..World::default()
        },
        World {
            capturable: false,
            ..World::default()
        },
    ] {
        let label = format!("{:?}/{}", world.field_class, world.capturable);
        let r = Rig::new(world, Duration::from_millis(5));
        r.h.send(Command::Toggle);
        assert_eq!(
            r.outcome(),
            Outcome::DraftReady(DraftReason::ReviewMode),
            "{label}"
        );
        assert!(r.pastes().is_empty(), "{label}");
        assert_eq!(
            r.clipboard(),
            None,
            "copy-only never writes the clipboard on its own ({label})"
        );
    }
}

#[test]
fn terminals_and_user_exclusions_are_copy_only() {
    let r = Rig::new(
        World {
            app_id: Some("com.googlecode.iterm2".into()),
            ..World::default()
        },
        Duration::from_millis(5),
    );
    r.h.send(Command::Toggle);
    assert_eq!(r.outcome(), Outcome::DraftReady(DraftReason::ReviewMode));
    assert!(r.pastes().is_empty());

    let r = Rig::new(
        World {
            app_id: Some("com.example.Secret".into()),
            ..World::default()
        },
        Duration::from_millis(5),
    );
    r.excluded.lock().unwrap().push("com.example.secret".into());
    r.h.send(Command::Toggle);
    assert_eq!(r.outcome(), Outcome::DraftReady(DraftReason::ReviewMode));
    assert!(r.pastes().is_empty());
}

#[test]
fn held_modifiers_time_out_into_draft_without_clipboard_write() {
    let r = Rig::new(
        World {
            modifiers_held_until: Some(Instant::now() + Duration::from_secs(30)),
            ..World::default()
        },
        Duration::from_millis(5),
    );
    r.h.send(Command::Toggle);
    assert_eq!(
        r.outcome(),
        Outcome::DraftReady(DraftReason::Blocked(AppError::ModifiersHeld))
    );
    assert!(r.pastes().is_empty());
    assert_eq!(r.clipboard(), None);
}

#[test]
fn modifiers_released_within_timeout_still_paste() {
    let r = Rig::new(World::default(), Duration::from_millis(5));
    r.world.lock().unwrap().modifiers_held_until =
        Some(Instant::now() + Duration::from_millis(1500));
    r.h.send(Command::Toggle);
    assert_eq!(r.outcome(), Outcome::PasteSent);
    assert_eq!(r.pastes().len(), 1);
}

#[test]
fn clipboard_busy_blocks_input() {
    let r = Rig::new(
        World {
            clipboard_fails: true,
            ..World::default()
        },
        Duration::from_millis(5),
    );
    r.h.send(Command::Toggle);
    assert_eq!(
        r.outcome(),
        Outcome::DraftReady(DraftReason::Blocked(AppError::ClipboardBusy))
    );
    assert!(r.pastes().is_empty());
}

#[test]
fn focus_change_after_clipboard_write_skips_input() {
    let r = Rig::new(
        World {
            switch_on_write: Some(Target { app: 3, field: 1 }),
            ..World::default()
        },
        Duration::from_millis(5),
    );
    r.h.send(Command::Toggle);
    assert_eq!(
        r.outcome(),
        Outcome::DraftReady(DraftReason::Blocked(AppError::TargetChanged))
    );
    assert!(r.pastes().is_empty(), "no input after the recheck fails");
    assert_eq!(
        r.clipboard().as_deref(),
        Some(TEXT),
        "the clipboard may now hold the text; reported, not hidden"
    );
}

#[test]
fn cancel_while_waiting_for_modifiers_never_touches_clipboard() {
    let r = Rig::new(
        World {
            modifiers_held_until: Some(Instant::now() + Duration::from_secs(30)),
            ..World::default()
        },
        Duration::from_millis(5),
    );
    r.h.send(Command::Toggle);
    r.wait_phase(Phase::Delivering);
    std::thread::sleep(Duration::from_millis(100));
    r.h.send(Command::Cancel);
    assert_eq!(r.outcome(), Outcome::Canceled);
    r.world.lock().unwrap().modifiers_held_until = None; // user lets go after cancel
    std::thread::sleep(Duration::from_millis(300));
    assert!(r.pastes().is_empty());
    assert_eq!(r.clipboard(), None);
    assert_eq!(r.h.snapshot(T).unwrap().phase, Phase::Idle);
}

#[test]
fn uncertain_send_is_never_retried() {
    let r = Rig::new(
        World {
            send_result: DispatchOutcome::Uncertain,
            ..World::default()
        },
        Duration::from_millis(5),
    );
    r.h.send(Command::Toggle);
    assert_eq!(r.outcome(), Outcome::DraftReady(DraftReason::Uncertain));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(r.pastes().len(), 1, "exactly one attempt");
    assert!(r.h.draft(T).unwrap().attempted);
}

#[test]
fn explicit_paste_targets_the_newly_focused_field() {
    let r = Rig::new(World::default(), Duration::from_millis(300));
    r.h.send(Command::Toggle);
    r.wait_phase(Phase::Transcribing);
    r.world.lock().unwrap().focus = Target { app: 2, field: 7 };
    assert_eq!(
        r.outcome(),
        Outcome::DraftReady(DraftReason::Blocked(AppError::TargetChanged))
    );
    // The user focuses where they want the text and presses the paste-draft chord.
    let d = r.h.draft(T).unwrap();
    r.h.send(Command::DraftPaste {
        id: d.id,
        revision: d.revision,
    });
    assert_eq!(r.outcome(), Outcome::PasteSent);
    assert_eq!(
        r.pastes(),
        vec![(Target { app: 2, field: 7 }, Some(TEXT.into()))]
    );
    assert!(r.h.draft(T).is_none());
}

#[test]
fn explicit_paste_into_secure_field_is_refused() {
    let r = Rig::new(World::default(), Duration::from_millis(300));
    r.h.send(Command::Toggle);
    r.wait_phase(Phase::Transcribing);
    {
        let mut w = r.world.lock().unwrap();
        w.focus = Target { app: 2, field: 1 };
    }
    assert_eq!(
        r.outcome(),
        Outcome::DraftReady(DraftReason::Blocked(AppError::TargetChanged))
    );
    r.world.lock().unwrap().field_class = FieldClass::Secure;
    let d = r.h.draft(T).unwrap();
    r.h.send(Command::DraftPaste {
        id: d.id,
        revision: d.revision,
    });
    let o = r.outcome();
    assert!(
        matches!(o, Outcome::DraftReady(DraftReason::Blocked(_))),
        "{o:?}"
    );
    assert!(r.pastes().is_empty());
    assert!(r.h.draft(T).is_some(), "draft kept");
}

#[test]
fn copy_only_setting_disables_automatic_delivery() {
    let r = Rig::new(World::default(), Duration::from_millis(5));
    r.h.send(Command::SetAutoDelivery(false));
    r.h.send(Command::Toggle);
    assert_eq!(r.outcome(), Outcome::DraftReady(DraftReason::ReviewMode));
    assert!(r.pastes().is_empty());
    let d = r.h.draft(T).unwrap();
    r.h.send(Command::DraftCopy {
        id: d.id,
        revision: d.revision,
    });
    assert_eq!(r.outcome(), Outcome::DraftCopied);
    assert_eq!(r.clipboard().as_deref(), Some(TEXT));
}

#[test]
fn many_sessions_never_double_dispatch() {
    let r = Rig::new(World::default(), Duration::from_millis(2));
    for i in 0..12 {
        r.h.send(Command::Toggle);
        if i % 4 == 3 {
            r.wait_phase(Phase::Delivering);
            r.h.send(Command::Cancel);
        }
        let o = r.outcome();
        assert!(
            matches!(
                o,
                Outcome::PasteSent | Outcome::Canceled | Outcome::CancelTooLate
            ),
            "{o:?}"
        );
        if let Some(d) = r.h.snapshot(T).unwrap().draft {
            r.h.send(Command::DraftDiscard { id: d.id });
            r.outcome();
        }
    }
    let sent = r.pastes().len();
    assert!(
        (9..=12).contains(&sent),
        "{sent} pastes for 12 sessions with 3 cancels"
    );
}
