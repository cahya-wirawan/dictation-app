//! M0 T0.8: earshot VAD validation on real speech and non-speech, plus the
//! full pipeline at 48 kHz stereo with random block sizes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use audioadapter_buffers::direct::InterleavedSlice;
use dictation_audio::FRAME_SAMPLES;
use dictation_audio::detector::{DetectorConfig, EarshotDetector, SpeechDetector};
use dictation_audio::endpoint::{EndpointConfig, FinishReason, Outcome};
use dictation_audio::pipeline::Pipeline;
use rubato::{Fft, FixedSync, Resampler};

fn jfk() -> Vec<f32> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/whisper.cpp/samples/jfk.wav");
    let mut r = hound::WavReader::open(p).unwrap();
    r.samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect()
}

fn noise(n: usize, amp: f32, seed: u64) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

fn voiced_fraction(x: &[f32]) -> f32 {
    let mut d = EarshotDetector::new(DetectorConfig::default());
    let frames: Vec<_> = x.chunks_exact(FRAME_SAMPLES).collect();
    let v = frames
        .iter()
        .filter(|f| d.classify((**f).try_into().unwrap()).voiced)
        .count();
    v as f32 / frames.len() as f32
}

#[test]
fn earshot_separates_speech_from_non_speech() {
    let speech = voiced_fraction(&jfk());
    let silence = voiced_fraction(&vec![0.0; 16_000 * 3]);
    let hiss = voiced_fraction(&noise(16_000 * 3, 0.003, 1)); // ≈ -55 dBFS, below energy floor
    let loud_noise = voiced_fraction(&noise(16_000 * 3, 0.1, 2)); // ≈ -25 dBFS white noise
    let tone: Vec<f32> = (0..16_000 * 3)
        .map(|i| 0.2 * (i as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 16_000.0).sin())
        .collect();
    let tone = voiced_fraction(&tone);
    eprintln!(
        "M0 T0.8 earshot voiced fraction: jfk={speech:.3} silence={silence:.3} hiss={hiss:.3} white-noise={loud_noise:.3} 1kHz-tone={tone:.3}"
    );
    assert!(speech > 0.5, "speech fixture under-detected: {speech}");
    assert_eq!(silence, 0.0);
    assert_eq!(hiss, 0.0);
    // Recorded, not tuned: the release no-speech suite (docs/08 §3) decides thresholds.
    assert!(
        loud_noise < 0.2,
        "white noise false-accept rate {loud_noise}"
    );
}

fn upsample_to_48k_stereo(x: &[f32]) -> Vec<f32> {
    let mut r = Fft::<f32>::new(16_000, 48_000, 1024, 1, FixedSync::Both).unwrap();
    let input = InterleavedSlice::new(x, 1, x.len()).unwrap();
    let need = r.process_all_needed_output_len(x.len());
    let mut out = vec![0.0; need];
    let mut o = InterleavedSlice::new_mut(&mut out, 1, need).unwrap();
    let (_, produced) = r
        .process_all_into_buffer(&input, &mut o, x.len(), None)
        .unwrap();
    out.truncate(produced);
    out.iter().flat_map(|s| [*s, *s]).collect()
}

fn run_48k(silence_ms: u32) -> (Outcome, Pipeline) {
    let mut audio = vec![0.0f32; 48_000 * 2 / 2]; // 0.5 s silence lead-in (stereo interleaved)
    audio.extend(upsample_to_48k_stereo(&jfk()));
    audio.extend(vec![0.0f32; 48_000 * 2 * 2]); // 2 s silence
    let cfg = EndpointConfig {
        silence_ms,
        ..Default::default()
    };
    let mut p = Pipeline::new(
        48_000,
        2,
        Box::new(EarshotDetector::new(DetectorConfig::default())),
        cfg,
    )
    .unwrap();
    let mut pos = 0;
    let mut seed = 11u64;
    while pos < audio.len() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let frames = ((seed >> 33) % 2048 + 1) as usize;
        let end = (pos + frames * 2).min(audio.len());
        if let Some(o) = p.push(&audio[pos..end]).unwrap() {
            return (o, p);
        }
        pos = end;
    }
    panic!("pipeline never finished");
}

/// jfk.wav contains two deliberate ~1.03 s pauses (measured with the
/// `vad_runs` example). The 1000 ms default therefore ends the phrase after
/// "And so my fellow Americans" — correct per spec, and a tuning data point.
#[test]
fn pipeline_48k_stereo_default_splits_at_oratorical_pause() {
    let (o, mut p) = run_48k(1000);
    let Outcome::Utterance {
        samples,
        reason,
        speech_ms,
    } = o
    else {
        panic!("{o:?}")
    };
    assert_eq!(reason, FinishReason::Silence);
    let secs = samples.len() as f32 / 16_000.0;
    eprintln!("pipeline(1000 ms): utterance {secs:.2}s, speech span {speech_ms} ms");
    assert!((2.0..=3.0).contains(&secs), "{secs}");
    assert!(
        p.push(&[0.0, 0.0]).is_err(),
        "finished pipeline rejects more audio"
    );
}

#[test]
fn pipeline_48k_stereo_longer_silence_keeps_whole_clip() {
    let (o, _) = run_48k(1500);
    let Outcome::Utterance {
        samples,
        reason,
        speech_ms,
    } = o
    else {
        panic!("{o:?}")
    };
    assert_eq!(reason, FinishReason::Silence);
    let secs = samples.len() as f32 / 16_000.0;
    eprintln!("pipeline(1500 ms): utterance {secs:.2}s, speech span {speech_ms} ms");
    // 0.3 s pre-roll + speech from 0.34 s to ~11.0 s + 0.2 s trailing pad.
    assert!((10.5..=11.6).contains(&secs), "{secs}");
}

#[test]
fn pipeline_stop_flushes_tail_once() {
    let mut p = Pipeline::new(
        44_100,
        1,
        Box::new(EarshotDetector::new(DetectorConfig::default())),
        EndpointConfig::default(),
    )
    .unwrap();
    assert!(p.push(&vec![0.0; 44_100]).unwrap().is_none());
    assert_eq!(p.stop().unwrap(), Outcome::NoSpeech);
    assert!(p.stop().is_err());
}
