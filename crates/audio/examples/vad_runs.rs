//! Diagnostic: print voiced/unvoiced run lengths (ms) for a 16 kHz mono WAV.
//!     cargo run -p dictation-audio --example vad_runs -- path.wav
use dictation_audio::detector::{DetectorConfig, EarshotDetector, SpeechDetector};
use dictation_audio::{FRAME_MS, FRAME_SAMPLES};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: vad_runs <16k-mono.wav>")?;
    let mut r = hound::WavReader::open(path)?;
    let x: Vec<f32> = r
        .samples::<i16>()
        .map(|s| s.map(|s| f32::from(s) / 32768.0))
        .collect::<Result<_, _>>()?;
    let mut d = EarshotDetector::new(DetectorConfig::default());
    let mut runs: Vec<(bool, u32)> = vec![];
    for f in x.chunks_exact(FRAME_SAMPLES) {
        let v = d.classify(f.try_into()?).voiced;
        match runs.last_mut() {
            Some((pv, n)) if *pv == v => *n += FRAME_MS,
            _ => runs.push((v, FRAME_MS)),
        }
    }
    let mut t = 0;
    for (v, n) in runs {
        println!(
            "{:>6} ms  {} {n} ms",
            t,
            if v { "VOICE " } else { "silent" }
        );
        t += n;
    }
    Ok(())
}
