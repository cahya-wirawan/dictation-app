//! Deterministic endpointing fixtures (docs/08 §2, work package A3).
//! Frames are 16 ms; durations below are expressed in frames.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use dictation_audio::endpoint::*;
use dictation_audio::{FRAME_MS, FRAME_SAMPLES};

const SPEECH: [f32; FRAME_SAMPLES] = [0.3; FRAME_SAMPLES];
const QUIET: [f32; FRAME_SAMPLES] = [0.0; FRAME_SAMPLES];

fn ms(n: u32) -> u32 {
    n.div_ceil(FRAME_MS)
}

/// Feed `(voiced, frames)` runs; return the first finished outcome and the frame index.
fn run(ep: &mut Endpointer, script: &[(bool, u32)]) -> Option<(Outcome, u32)> {
    let mut i = 0;
    for &(voiced, n) in script {
        for _ in 0..n {
            i += 1;
            let f = if voiced { &SPEECH } else { &QUIET };
            if let Event::Finished(o) = ep.push(f, voiced).unwrap() {
                return Some((o, i));
            }
        }
    }
    None
}

fn secs(samples: &[f32]) -> f32 {
    samples.len() as f32 / 16_000.0
}

#[test]
fn pause_shorter_than_silence_keeps_one_phrase() {
    let mut ep = Endpointer::new(EndpointConfig::default());
    let r = run(
        &mut ep,
        &[
            (true, ms(2000)),
            (false, ms(900)),
            (true, ms(2000)),
            (false, ms(1100)),
        ],
    );
    let (
        Outcome::Utterance {
            reason, speech_ms, ..
        },
        _,
    ) = r.unwrap()
    else {
        panic!()
    };
    assert_eq!(reason, FinishReason::Silence);
    assert!(
        speech_ms >= 4800,
        "both speech runs plus the pause: {speech_ms}"
    );
}

#[test]
fn silence_finishes_once_with_trailing_pad() {
    let mut ep = Endpointer::new(EndpointConfig::default());
    let speech = ms(2000);
    let (o, at) = run(&mut ep, &[(true, speech), (false, ms(3000))]).unwrap();
    // Finishes exactly when the unvoiced run reaches 1000 ms.
    assert_eq!(at, speech + ms(1000));
    let Outcome::Utterance {
        samples, reason, ..
    } = o
    else {
        panic!()
    };
    assert_eq!(reason, FinishReason::Silence);
    // speech + 200 ms trailing pad (onset triggers on the 3rd voiced frame,
    // and the two earlier voiced frames are in the pre-roll).
    let expected = (speech + ms(200)) as usize * FRAME_SAMPLES;
    assert_eq!(samples.len(), expected, "{}s", secs(&samples));
    assert!(ep.push(&QUIET, false).is_err(), "no events after finish");
}

#[test]
fn no_speech_times_out_without_audio() {
    let mut ep = Endpointer::new(EndpointConfig::default());
    let (o, at) = run(&mut ep, &[(false, ms(10_000))]).unwrap();
    assert_eq!(o, Outcome::NoSpeech);
    assert_eq!(at, ms(8000));
}

#[test]
fn isolated_impulse_is_not_onset() {
    let mut ep = Endpointer::new(EndpointConfig::default());
    // 1-2 voiced frames separated by enough silence never reach 3-of-5.
    let script = [
        (false, 20),
        (true, 1),
        (false, 4),
        (true, 2),
        (false, 4),
        (true, 1),
        (false, ms(8000)),
    ];
    let (o, _) = run(&mut ep, &script).unwrap();
    assert_eq!(o, Outcome::NoSpeech);
}

#[test]
fn preroll_keeps_audio_before_onset() {
    let mut ep = Endpointer::new(EndpointConfig::default());
    // 1 s quiet then speech: the utterance must start ~300 ms before the 3rd voiced frame.
    let (o, _) = run(
        &mut ep,
        &[(false, ms(1000)), (true, ms(1000)), (false, ms(1000))],
    )
    .unwrap();
    let Outcome::Utterance { samples, .. } = o else {
        panic!()
    };
    let leading_quiet = samples.iter().take_while(|s| **s == 0.0).count() / FRAME_SAMPLES;
    // Pre-roll holds 19 frames (304 ms) including the first 3 voiced frames → 16 quiet frames.
    assert_eq!(leading_quiet as u32, ms(300) - 3);
}

#[test]
fn short_burst_is_too_short() {
    let mut ep = Endpointer::new(EndpointConfig::default());
    let (o, _) = run(&mut ep, &[(true, 5), (false, ms(1100))]).unwrap();
    assert_eq!(o, Outcome::TooShort);
}

#[test]
fn cap_finalizes_exactly_once() {
    let cfg = EndpointConfig {
        max_capture_ms: 2000,
        ..Default::default()
    };
    let mut ep = Endpointer::new(cfg);
    let (o, at) = run(&mut ep, &[(true, ms(5000))]).unwrap();
    assert_eq!(at, ms(2000));
    let Outcome::Utterance {
        reason, samples, ..
    } = o
    else {
        panic!()
    };
    assert_eq!(reason, FinishReason::Cap);
    assert_eq!(samples.len(), ms(2000) as usize * FRAME_SAMPLES);
    assert_eq!(ep.stop(), Err(AlreadyFinished));
}

#[test]
fn default_cap_is_120s() {
    let mut ep = Endpointer::new(EndpointConfig::default());
    let (o, at) = run(&mut ep, &[(true, ms(130_000))]).unwrap();
    assert_eq!(at, ms(120_000));
    let Outcome::Utterance { samples, .. } = o else {
        panic!()
    };
    assert!(
        samples.len() <= 1_920_000 + FRAME_SAMPLES,
        "within worker MAX_SAMPLES"
    );
}

#[test]
fn explicit_stop_uses_available_audio_and_is_idempotent() {
    let mut ep = Endpointer::new(EndpointConfig::default());
    assert!(run(&mut ep, &[(true, ms(1000)), (false, ms(400))]).is_none());
    let Ok(Outcome::Utterance {
        reason, samples, ..
    }) = ep.stop()
    else {
        panic!()
    };
    assert_eq!(reason, FinishReason::Stop);
    assert_eq!(samples.len(), (ms(1000) + ms(400)) as usize * FRAME_SAMPLES);
    // Stop/endpoint race: the second finish request loses.
    assert_eq!(ep.stop(), Err(AlreadyFinished));
    assert!(ep.push(&QUIET, false).is_err());
}

#[test]
fn stop_before_speech_is_no_speech() {
    let mut ep = Endpointer::new(EndpointConfig::default());
    assert!(run(&mut ep, &[(false, 30), (true, 2)]).is_none());
    assert_eq!(ep.stop(), Ok(Outcome::NoSpeech));
}
