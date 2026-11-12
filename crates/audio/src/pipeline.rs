//! DSP worker logic: interleaved native audio → mono → 16 kHz → frames →
//! detector → endpointer. Single-threaded and deterministic; the caller owns
//! scheduling (drain ring, push, emit meter at ≤15 Hz).

use crate::FRAME_SAMPLES;
use crate::convert::{ConvertError, dbfs, downmix_into, rms};
use crate::detector::SpeechDetector;
use crate::endpoint::{AlreadyFinished, EndpointConfig, Endpointer, Event, Outcome};
use crate::resample::{ResampleError, StreamResampler};

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error(transparent)]
    Convert(#[from] ConvertError),
    #[error(transparent)]
    Resample(#[from] ResampleError),
    #[error(transparent)]
    Finished(#[from] AlreadyFinished),
}

pub struct Pipeline {
    channels: usize,
    resampler: StreamResampler,
    detector: Box<dyn SpeechDetector>,
    endpointer: Endpointer,
    mono: Vec<f32>,
    rs_out: Vec<f32>,
    frame: Vec<f32>,
    level: f32,
    onset: bool,
    outcome: Option<Outcome>,
}

impl Pipeline {
    pub fn new(
        native_rate: u32,
        channels: u16,
        detector: Box<dyn SpeechDetector>,
        cfg: EndpointConfig,
    ) -> Result<Self, PipelineError> {
        Ok(Self {
            channels: usize::from(channels),
            resampler: StreamResampler::new(native_rate)?,
            detector,
            endpointer: Endpointer::new(cfg),
            mono: Vec::with_capacity(8192),
            rs_out: Vec::with_capacity(4096),
            frame: Vec::with_capacity(FRAME_SAMPLES),
            level: 0.0,
            onset: false,
            outcome: None,
        })
    }

    /// Normalized meter level in [0, 1] from the most recent frame (−60..0 dBFS).
    pub fn level(&self) -> f32 {
        self.level
    }
    pub fn has_onset(&self) -> bool {
        self.onset
    }

    /// Feed interleaved native samples. Returns the outcome once finished.
    pub fn push(&mut self, interleaved: &[f32]) -> Result<Option<Outcome>, PipelineError> {
        if self.outcome.is_some() || self.endpointer.is_done() {
            return Err(AlreadyFinished.into());
        }
        self.mono.clear();
        downmix_into(interleaved, self.channels, &mut self.mono)?;
        self.rs_out.clear();
        self.resampler.push(&self.mono, &mut self.rs_out)?;
        let out = std::mem::take(&mut self.rs_out);
        let r = self.feed_frames(&out);
        self.rs_out = out;
        r
    }

    /// Explicit stop: flush the resampler tail once and finish with what we have.
    pub fn stop(&mut self) -> Result<Outcome, PipelineError> {
        if let Some(o) = self.outcome.take() {
            return Ok(o);
        }
        if self.endpointer.is_done() {
            return Err(AlreadyFinished.into());
        }
        let mut tail = Vec::new();
        self.resampler.flush(&mut tail)?;
        if let Some(o) = self.feed_frames(&tail)? {
            return Ok(o);
        }
        // Drop the incomplete final frame (<16 ms) rather than pad with fake silence.
        self.frame.clear();
        Ok(self.endpointer.stop()?)
    }

    fn feed_frames(&mut self, samples: &[f32]) -> Result<Option<Outcome>, PipelineError> {
        for &s in samples {
            self.frame.push(s);
            if self.frame.len() == FRAME_SAMPLES {
                let mut buf = [0.0f32; FRAME_SAMPLES];
                buf.copy_from_slice(&self.frame);
                self.frame.clear();
                self.level = ((dbfs(rms(&buf)) + 60.0) / 60.0).clamp(0.0, 1.0);
                let class = self.detector.classify(&buf);
                match self.endpointer.push(&buf, class.voiced)? {
                    Event::Continue => {}
                    Event::Onset => self.onset = true,
                    Event::Finished(o) => return Ok(Some(o)),
                }
            }
        }
        Ok(None)
    }
}
