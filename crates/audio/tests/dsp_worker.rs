//! DSP worker (A2): fault injection, explicit stop/abort, meter rate, and the
//! "microphone closed before any terminal event" guarantee.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dictation_audio::capture::{CaptureError, NativeFormat};
use dictation_audio::detector::{DetectorConfig, EarshotDetector};
use dictation_audio::dsp::*;
use dictation_audio::endpoint::{EndpointConfig, FinishReason, Outcome};

/// Plays scripted interleaved audio, `chunk` samples per drain; can fail at a
/// given drain index. Records whether it was closed and how often.
struct FakeMic {
    data: Vec<f32>,
    pos: usize,
    chunk: usize,
    fail_at: Option<(usize, CaptureError)>,
    drains: usize,
    format: NativeFormat,
    closed: Arc<AtomicBool>,
    close_calls: Arc<AtomicUsize>,
}

impl AudioSource for FakeMic {
    fn format(&self) -> NativeFormat {
        self.format
    }
    fn drain_into(&mut self, out: &mut Vec<f32>) -> Result<usize, CaptureError> {
        assert!(!self.closed.load(Ordering::SeqCst), "drain after close");
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
        self.closed.store(true, Ordering::SeqCst);
        self.close_calls.fetch_add(1, Ordering::SeqCst);
    }
}

fn jfk_16k() -> Vec<f32> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/whisper.cpp/samples/jfk.wav");
    hound::WavReader::open(p)
        .unwrap()
        .samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect()
}

struct Run {
    events: Vec<(Instant, DspEvent)>,
    closed_at_terminal: bool,
    close_calls: usize,
}

fn run(
    data: Vec<f32>,
    chunk: usize,
    fail_at: Option<(usize, CaptureError)>,
    cfg: EndpointConfig,
    control: impl FnOnce(&DspSession),
) -> Run {
    let closed = Arc::new(AtomicBool::new(false));
    let close_calls = Arc::new(AtomicUsize::new(0));
    let mic = FakeMic {
        data,
        pos: 0,
        chunk,
        fail_at,
        drains: 0,
        format: NativeFormat {
            sample_rate: 16_000,
            channels: 1,
            sample_format: "f32",
        },
        closed: Arc::clone(&closed),
        close_calls: Arc::clone(&close_calls),
    };
    let (tx, rx) = mpsc::channel();
    let closed_probe = Arc::clone(&closed);
    let session = DspSession::spawn(
        Box::new(mic),
        Box::new(EarshotDetector::new(DetectorConfig::default())),
        cfg,
        Duration::from_millis(2),
        move |ev| {
            let terminal = matches!(
                ev,
                DspEvent::Finished(_) | DspEvent::Fault(_) | DspEvent::Aborted
            );
            let _ = tx.send((
                Instant::now(),
                ev,
                terminal && closed_probe.load(Ordering::SeqCst),
            ));
        },
    )
    .unwrap();
    control(&session);
    let mut events = vec![];
    let mut closed_at_terminal = false;
    while let Ok((t, ev, closed_now)) = rx.recv_timeout(Duration::from_secs(20)) {
        let terminal = matches!(
            ev,
            DspEvent::Finished(_) | DspEvent::Fault(_) | DspEvent::Aborted
        );
        events.push((t, ev));
        if terminal {
            closed_at_terminal = closed_now;
            break;
        }
    }
    session.join();
    Run {
        events,
        closed_at_terminal,
        close_calls: close_calls.load(Ordering::SeqCst),
    }
}

fn terminal(r: &Run) -> &DspEvent {
    &r.events.last().unwrap().1
}

#[test]
fn speech_then_silence_finishes_with_mic_closed() {
    let mut data = jfk_16k();
    data.extend(vec![0.0; 16_000 * 2]);
    let r = run(
        data,
        1600,
        None,
        EndpointConfig {
            silence_ms: 1500,
            ..Default::default()
        },
        |_| {},
    );
    let DspEvent::Finished(Outcome::Utterance {
        samples, reason, ..
    }) = terminal(&r)
    else {
        panic!("{:?}", terminal(&r))
    };
    assert_eq!(*reason, FinishReason::Silence);
    assert!(samples.len() > 16_000 * 10);
    assert!(
        r.closed_at_terminal,
        "microphone must be closed before Finished"
    );
    assert_eq!(
        r.events
            .iter()
            .filter(|(_, e)| *e == DspEvent::Onset)
            .count(),
        1
    );
}

#[test]
fn meter_is_rate_limited() {
    let mut data = jfk_16k();
    data.extend(vec![0.0; 16_000 * 2]);
    // 160 samples (10 ms) per 2 ms poll: ~5x faster than real time.
    let r = run(
        data,
        160,
        None,
        EndpointConfig {
            silence_ms: 1500,
            ..Default::default()
        },
        |_| {},
    );
    let levels: Vec<Instant> = r
        .events
        .iter()
        .filter(|(_, e)| matches!(e, DspEvent::Level(_)))
        .map(|(t, _)| *t)
        .collect();
    assert!(levels.len() > 5);
    for w in levels.windows(2) {
        assert!(
            w[1] - w[0] >= METER_INTERVAL - Duration::from_millis(1),
            "meter faster than 15 Hz"
        );
    }
}

#[test]
fn explicit_stop_uses_available_audio() {
    // 3 s of speech delivered slowly; stop after ~1 s of wall time.
    let speech: Vec<f32> = jfk_16k()
        .into_iter()
        .skip(16_000 / 3)
        .take(16_000 * 3)
        .collect();
    let r = run(speech, 64, None, EndpointConfig::default(), |s| {
        std::thread::sleep(Duration::from_millis(1200));
        s.stop();
        s.stop(); // idempotent
    });
    let DspEvent::Finished(Outcome::Utterance { reason, .. }) = terminal(&r) else {
        panic!("{:?}", terminal(&r))
    };
    assert_eq!(*reason, FinishReason::Stop);
    assert!(r.closed_at_terminal);
}

#[test]
fn stop_before_speech_is_no_speech() {
    let r = run(
        vec![0.0; 16_000 * 30],
        160,
        None,
        EndpointConfig::default(),
        |s| {
            std::thread::sleep(Duration::from_millis(100));
            s.stop();
        },
    );
    assert_eq!(terminal(&r), &DspEvent::Finished(Outcome::NoSpeech));
    assert!(r.closed_at_terminal);
}

#[test]
fn abort_discards_and_closes() {
    let r = run(jfk_16k(), 64, None, EndpointConfig::default(), |s| {
        std::thread::sleep(Duration::from_millis(100));
        s.abort();
    });
    assert_eq!(terminal(&r), &DspEvent::Aborted);
    assert!(r.closed_at_terminal);
    assert!(
        !r.events
            .iter()
            .any(|(_, e)| matches!(e, DspEvent::Finished(_)))
    );
}

#[test]
fn overflow_and_disconnect_abort_without_partial_success() {
    for err in [CaptureError::Overflow, CaptureError::Disconnected] {
        // Fault at 1.5 s: mid-speech, before the first natural pause in jfk.wav.
        let r = run(
            jfk_16k(),
            1600,
            Some((15, err)),
            EndpointConfig::default(),
            |_| {},
        );
        assert!(
            r.events.iter().any(|(_, e)| *e == DspEvent::Onset),
            "fault must hit during speech"
        );
        assert_eq!(
            terminal(&r),
            &DspEvent::Fault(DspFault::Capture(err)),
            "{err:?}"
        );
        assert!(r.closed_at_terminal);
        assert!(
            !r.events
                .iter()
                .any(|(_, e)| matches!(e, DspEvent::Finished(_))),
            "no truncated utterance"
        );
    }
}

#[test]
fn non_finite_samples_fault() {
    let mut data = vec![0.0; 16_000];
    data[8000] = f32::NAN;
    let r = run(data, 1600, None, EndpointConfig::default(), |_| {});
    assert_eq!(terminal(&r), &DspEvent::Fault(DspFault::InvalidAudio));
    assert!(r.closed_at_terminal);
}

#[test]
fn dropping_the_session_closes_the_microphone() {
    let closed = Arc::new(AtomicBool::new(false));
    let mic = FakeMic {
        data: vec![0.0; 16_000 * 60],
        pos: 0,
        chunk: 16,
        fail_at: None,
        drains: 0,
        format: NativeFormat {
            sample_rate: 48_000,
            channels: 2,
            sample_format: "f32",
        },
        closed: Arc::clone(&closed),
        close_calls: Arc::new(AtomicUsize::new(0)),
    };
    let s = DspSession::spawn(
        Box::new(mic),
        Box::new(EarshotDetector::new(DetectorConfig::default())),
        EndpointConfig::default(),
        Duration::from_millis(2),
        |_| {},
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    drop(s);
    assert!(
        closed.load(Ordering::SeqCst),
        "no idle capture after the session is dropped"
    );
}

#[test]
fn close_is_called_exactly_once_on_terminal_paths() {
    let r = run(
        vec![0.0; 16_000 * 9],
        1600,
        None,
        EndpointConfig::default(),
        |_| {},
    );
    assert_eq!(terminal(&r), &DspEvent::Finished(Outcome::NoSpeech));
    assert_eq!(r.close_calls, 1);
}
