//! Per-frame speech classification behind a replaceable trait (docs/03 §4).
//! A frame is voiced only if the VAD score passes *and* the frame clears a
//! conservative energy floor, so near-silent hiss cannot start a session.

use crate::FRAME_SAMPLES;
use crate::convert::{dbfs, rms};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameClass {
    pub voiced: bool,
    pub score: f32,
    pub rms_dbfs: f32,
}

pub trait SpeechDetector: Send {
    fn classify(&mut self, frame: &[f32; FRAME_SAMPLES]) -> FrameClass;
    fn reset(&mut self);
}

#[derive(Debug, Clone, Copy)]
pub struct DetectorConfig {
    pub score_threshold: f32,
    pub energy_floor_dbfs: f32,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        // Proposals; to be calibrated against the tuning corpus (docs/08 §3).
        Self {
            score_threshold: 0.5,
            energy_floor_dbfs: -50.0,
        }
    }
}

/// earshot 1.2 neural VAD (pure Rust, MIT/Apache-2.0). See ADR-0001.
pub struct EarshotDetector {
    vad: Box<earshot::Detector>,
    cfg: DetectorConfig,
}

impl EarshotDetector {
    pub fn new(cfg: DetectorConfig) -> Self {
        Self {
            vad: earshot::Detector::default_boxed(),
            cfg,
        }
    }
}

impl SpeechDetector for EarshotDetector {
    fn classify(&mut self, frame: &[f32; FRAME_SAMPLES]) -> FrameClass {
        let score = self.vad.predict_f32(frame);
        let rms_dbfs = dbfs(rms(frame));
        FrameClass {
            voiced: score >= self.cfg.score_threshold && rms_dbfs >= self.cfg.energy_floor_dbfs,
            score,
            rms_dbfs,
        }
    }
    fn reset(&mut self) {
        self.vad.reset();
    }
}

/// Deterministic detector for tests: voiced iff RMS above a floor.
pub struct EnergyDetector {
    pub floor_dbfs: f32,
}

impl SpeechDetector for EnergyDetector {
    fn classify(&mut self, frame: &[f32; FRAME_SAMPLES]) -> FrameClass {
        let rms_dbfs = dbfs(rms(frame));
        FrameClass {
            voiced: rms_dbfs >= self.floor_dbfs,
            score: f32::from(u8::from(rms_dbfs >= self.floor_dbfs)),
            rms_dbfs,
        }
    }
    fn reset(&mut self) {}
}
