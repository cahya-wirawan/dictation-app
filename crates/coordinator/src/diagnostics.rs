//! Redacted support diagnostics (docs/10 §5, docs/06).
//!
//! Records only *codes* and *durations*: outcome/error codes from the
//! notifier and per-job audio/inference milliseconds from the transcriber.
//! Nothing here ever receives transcript text, audio, target identities,
//! device names or file paths, so an export cannot leak them. The report is
//! built on request and never uploaded; the app shows a preview and writes it
//! only on an explicit Save.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use dictation_domain::Phase;
use dictation_domain::session::Outcome;
use serde::Serialize;

use crate::actor::Notifier;

/// Bounded so a long-running tray app keeps constant memory.
pub const MAX_EVENTS: usize = 64;
pub const MAX_TIMINGS: usize = 100;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiagEvent {
    /// Seconds since the Unix epoch (coarse; no sub-second correlation).
    pub at: u64,
    /// Serialized outcome code, e.g. `"pasteSent"` or `{"failed":"WORKER_CRASH"}`.
    pub code: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobTiming {
    pub audio_ms: u32,
    pub inference_ms: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimingSummary {
    pub jobs: usize,
    pub inference_ms_median: Option<u32>,
    pub inference_ms_p95: Option<u32>,
    pub inference_ms_max: Option<u32>,
    /// Inference seconds per audio second (median over jobs).
    pub real_time_factor_median: Option<f64>,
}

#[derive(Default)]
struct Inner {
    events: VecDeque<DiagEvent>,
    timings: VecDeque<JobTiming>,
    worker_restarts: u64,
    worker: Option<WorkerFacts>,
}

/// What the running worker reported in its Hello handshake.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerFacts {
    pub engine: String,
    pub engine_revision: String,
    pub backends: Vec<String>,
}

/// Shared, thread-safe recorder. Cheap to clone via `Arc`.
#[derive(Default)]
pub struct Diagnostics {
    inner: Mutex<Inner>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Diagnostics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn record_outcome(&self, outcome: Outcome) {
        let code = serde_json::to_value(outcome).unwrap_or(serde_json::Value::Null);
        if let Ok(mut i) = self.inner.lock() {
            if i.events.len() == MAX_EVENTS {
                i.events.pop_front();
            }
            i.events.push_back(DiagEvent {
                at: now_secs(),
                code,
            });
        }
    }

    pub fn record_timing(&self, t: JobTiming) {
        if let Ok(mut i) = self.inner.lock() {
            if i.timings.len() == MAX_TIMINGS {
                i.timings.pop_front();
            }
            i.timings.push_back(t);
        }
    }

    pub fn record_worker(&self, w: WorkerFacts) {
        if let Ok(mut i) = self.inner.lock() {
            i.worker = Some(w);
        }
    }

    pub fn worker(&self) -> Option<WorkerFacts> {
        self.inner.lock().ok().and_then(|i| i.worker.clone())
    }

    pub fn record_worker_restart(&self) {
        if let Ok(mut i) = self.inner.lock() {
            i.worker_restarts += 1;
        }
    }

    pub fn events(&self) -> Vec<DiagEvent> {
        self.inner
            .lock()
            .map(|i| i.events.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn worker_restarts(&self) -> u64 {
        self.inner.lock().map_or(0, |i| i.worker_restarts)
    }

    pub fn timing_summary(&self) -> TimingSummary {
        let timings: Vec<JobTiming> = self
            .inner
            .lock()
            .map(|i| i.timings.iter().copied().collect())
            .unwrap_or_default();
        summarize(&timings)
    }
}

/// Nearest-rank percentile over a sorted slice.
pub fn percentile(sorted: &[u32], p: f64) -> Option<u32> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.clamp(1, sorted.len()) - 1).copied()
}

pub fn summarize(timings: &[JobTiming]) -> TimingSummary {
    let mut inf: Vec<u32> = timings.iter().map(|t| t.inference_ms).collect();
    inf.sort_unstable();
    let mut rtf: Vec<f64> = timings
        .iter()
        .filter(|t| t.audio_ms > 0)
        .map(|t| f64::from(t.inference_ms) / f64::from(t.audio_ms))
        .collect();
    rtf.sort_by(f64::total_cmp);
    TimingSummary {
        jobs: timings.len(),
        inference_ms_median: percentile(&inf, 50.0),
        inference_ms_p95: percentile(&inf, 95.0),
        inference_ms_max: inf.last().copied(),
        real_time_factor_median: (!rtf.is_empty()).then(|| rtf[(rtf.len() - 1) / 2]),
    }
}

/// Wraps the real notifier and records every outcome code on the way past.
pub struct RecordingNotifier<N> {
    pub inner: N,
    pub diagnostics: Arc<Diagnostics>,
}

impl<N: Notifier> Notifier for RecordingNotifier<N> {
    fn phase(&mut self, phase: Phase) {
        self.inner.phase(phase);
    }
    fn outcome(&mut self, outcome: Outcome) {
        self.diagnostics.record_outcome(outcome);
        self.inner.outcome(outcome);
    }
    fn level(&mut self, level: f32) {
        self.inner.level(level);
    }
}

/// Non-sensitive app facts supplied by the shell. Every field is a version,
/// an identifier from the approved manifest, an OS status code or a number.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportFacts {
    pub app_version: String,
    pub os: &'static str,
    pub arch: &'static str,
    pub build_profile: &'static str,
    pub protocol_version: u32,
    pub active_model: Option<&'static str>,
    pub installed_models: Vec<&'static str>,
    pub permissions: serde_json::Value,
    pub readiness: Option<serde_json::Value>,
    pub settings: SettingsSummary,
}

/// Settings reduced to non-identifying values: no device names, no excluded
/// app identities (only their count), no file paths.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSummary {
    pub schema_version: u32,
    pub delivery_mode: serde_json::Value,
    pub language: String,
    pub silence_ms: u32,
    pub max_capture_ms: u32,
    pub custom_input_device: bool,
    pub excluded_app_count: usize,
    pub show_hud: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub report_version: u32,
    pub generated_at: u64,
    pub notice: &'static str,
    #[serde(flatten)]
    pub facts: ReportFacts,
    /// `None` until the first job ran in this app session.
    pub worker: Option<WorkerFacts>,
    pub worker_restarts: u64,
    pub timings: TimingSummary,
    pub recent_outcomes: Vec<DiagEvent>,
}

pub const REPORT_NOTICE: &str = "Contains versions, settings values, status codes and timings only. \
No dictated text, audio, app or window names, device names or file paths.";

impl Diagnostics {
    pub fn report(&self, facts: ReportFacts) -> Report {
        Report {
            report_version: 1,
            generated_at: now_secs(),
            notice: REPORT_NOTICE,
            facts,
            worker: self.worker(),
            worker_restarts: self.worker_restarts(),
            timings: self.timing_summary(),
            recent_outcomes: self.events(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dictation_domain::session::{AppError, DraftReason};

    #[test]
    fn rings_are_bounded() {
        let d = Diagnostics::new();
        for _ in 0..(MAX_EVENTS * 3) {
            d.record_outcome(Outcome::PasteSent);
        }
        for i in 0..(MAX_TIMINGS * 3) {
            d.record_timing(JobTiming {
                audio_ms: 1000,
                inference_ms: u32::try_from(i).unwrap_or(0),
            });
        }
        assert_eq!(d.events().len(), MAX_EVENTS);
        assert_eq!(d.timing_summary().jobs, MAX_TIMINGS);
    }

    #[test]
    fn codes_are_serialized_without_payload_text() {
        let d = Diagnostics::new();
        d.record_outcome(Outcome::Failed(AppError::WorkerCrash));
        d.record_outcome(Outcome::DraftReady(DraftReason::Blocked(
            AppError::TargetChanged,
        )));
        let s = serde_json::to_string(&d.events()).unwrap_or_default();
        assert!(s.contains("WORKER_CRASH"), "{s}");
        assert!(s.contains("TARGET_CHANGED"), "{s}");
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let v: Vec<u32> = (1..=20).collect();
        assert_eq!(percentile(&v, 50.0), Some(10));
        assert_eq!(percentile(&v, 95.0), Some(19));
        assert_eq!(percentile(&v, 100.0), Some(20));
        assert_eq!(percentile(&[], 50.0), None);
        let s = summarize(&[
            JobTiming {
                audio_ms: 10_000,
                inference_ms: 500,
            },
            JobTiming {
                audio_ms: 0,
                inference_ms: 7,
            },
        ]);
        assert_eq!(s.inference_ms_max, Some(500));
        assert_eq!(s.real_time_factor_median, Some(0.05));
    }
}
