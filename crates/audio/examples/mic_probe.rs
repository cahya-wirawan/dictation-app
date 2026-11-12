//! M0 T0.2 manual probe: lists input devices, captures N seconds from the
//! default (or `--device <id>`) input, prints native format, peak level and
//! endpointing outcome. Run on the target machine inside a signed bundle to
//! exercise microphone permission:
//!
//!     cargo run -p dictation-audio --example mic_probe -- [--device ID] [--seconds 5]

use std::time::{Duration, Instant};

use dictation_audio::capture::{self, CaptureError};
use dictation_audio::detector::{DetectorConfig, EarshotDetector};
use dictation_audio::endpoint::{EndpointConfig, Outcome};
use dictation_audio::pipeline::Pipeline;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let seconds: u64 = arg("--seconds").and_then(|s| s.parse().ok()).unwrap_or(5);

    match capture::list_input_devices() {
        Ok(devs) => {
            for d in devs {
                println!(
                    "{} {:40} {}",
                    if d.is_default { "*" } else { " " },
                    d.name,
                    d.id
                );
            }
        }
        Err(e) => println!("device enumeration failed: {e}"),
    }

    let (mut handle, mut reader) = match capture::start(arg("--device").as_deref()) {
        Ok(v) => v,
        Err(CaptureError::PermissionDenied) => {
            println!("microphone permission denied");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    let f = reader.format;
    println!(
        "capturing {seconds}s: {} Hz, {} ch, {}",
        f.sample_rate, f.channels, f.sample_format
    );
    let mut pipe = Pipeline::new(
        f.sample_rate,
        f.channels,
        Box::new(EarshotDetector::new(DetectorConfig::default())),
        EndpointConfig::default(),
    )?;
    let started = Instant::now();
    let mut buf = Vec::new();
    let mut last_meter = Instant::now();
    let mut outcome = None;
    while started.elapsed() < Duration::from_secs(seconds) {
        buf.clear();
        reader.drain_into(&mut buf)?;
        if let Some(o) = pipe.push(&buf)? {
            outcome = Some(o);
            break;
        }
        if last_meter.elapsed() >= Duration::from_millis(67) {
            let bars = (pipe.level() * 40.0) as usize;
            print!("\r[{:40}] onset={}", "#".repeat(bars), pipe.has_onset());
            last_meter = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    handle.stop(); // microphone closed here, before any further processing
    let outcome = match outcome {
        Some(o) => o,
        None => pipe.stop()?,
    };
    println!();
    match outcome {
        Outcome::Utterance {
            samples,
            reason,
            speech_ms,
        } => {
            println!(
                "utterance: {:.2}s audio, speech span {speech_ms} ms, reason {reason:?}",
                samples.len() as f32 / 16000.0
            )
        }
        other => println!("outcome: {other:?}"),
    }
    Ok(())
}
