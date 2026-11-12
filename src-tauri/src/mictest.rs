//! Exclusive microphone test (docs/04 §3 start_mic_test/stop_mic_test).
//! The microphone is open only while the test runs; levels go to the settings
//! window only, at ≤15 Hz. No audio is stored.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dictation_audio::capture;
use dictation_audio::convert::{dbfs, downmix_into, rms};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

const MAX_TEST: Duration = Duration::from_secs(30);
const METER_INTERVAL: Duration = Duration::from_millis(67);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicLevel {
    pub level: f32,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicTestEnded {
    pub reason: String,
}

pub struct MicTest {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl MicTest {
    pub fn start(app: AppHandle, device_id: Option<String>) -> Result<Self, String> {
        let (mut handle, mut reader) =
            capture::start(device_id.as_deref()).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = Arc::clone(&stop);
        let join = std::thread::Builder::new()
            .name("mic-test-dsp".into())
            .spawn(move || {
                let f = reader.format;
                let started = Instant::now();
                let mut last_emit = Instant::now();
                let mut buf = Vec::new();
                let mut mono = Vec::new();
                let reason = loop {
                    if stop_t.load(Ordering::Acquire) {
                        break "stopped".to_string();
                    }
                    if started.elapsed() > MAX_TEST {
                        break "timeLimit".to_string();
                    }
                    buf.clear();
                    if let Err(e) = reader.drain_into(&mut buf) {
                        break format!("error: {e}");
                    }
                    if !buf.is_empty() && last_emit.elapsed() >= METER_INTERVAL {
                        mono.clear();
                        if downmix_into(&buf, usize::from(f.channels), &mut mono).is_err() {
                            break "error: invalid samples".to_string();
                        }
                        let level = ((dbfs(rms(&mono)) + 60.0) / 60.0).clamp(0.0, 1.0);
                        let _ = app.emit_to(
                            "main",
                            "mic_level",
                            MicLevel {
                                level,
                                sample_rate: f.sample_rate,
                                channels: f.channels,
                            },
                        );
                        last_emit = Instant::now();
                    }
                    std::thread::sleep(Duration::from_millis(15));
                };
                handle.stop(); // close the microphone before reporting
                let _ = app.emit_to("main", "mic_test_ended", MicTestEnded { reason });
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stop,
            join: Some(join),
        })
    }

    pub fn is_finished(&self) -> bool {
        self.join.as_ref().is_none_or(|j| j.is_finished())
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for MicTest {
    fn drop(&mut self) {
        self.stop();
    }
}
