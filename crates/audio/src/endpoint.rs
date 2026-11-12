//! Deterministic frame-count endpointing (docs/03 §4 steps 6–9).
//! Time is derived from frame counts only, never wall clock.

use std::collections::VecDeque;

use crate::{FRAME_MS, FRAME_SAMPLES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointConfig {
    pub preroll_ms: u32,
    pub onset_voiced: u32,
    pub onset_window: u32,
    pub silence_ms: u32,
    pub trailing_ms: u32,
    pub no_speech_timeout_ms: u32,
    pub max_capture_ms: u32,
    pub min_speech_ms: u32,
}

impl Default for EndpointConfig {
    fn default() -> Self {
        Self {
            preroll_ms: 300,
            onset_voiced: 3,
            onset_window: 5,
            silence_ms: 1000,
            trailing_ms: 200,
            no_speech_timeout_ms: 8000,
            max_capture_ms: 120_000,
            min_speech_ms: 200,
        }
    }
}

fn frames(ms: u32) -> usize {
    ms.div_ceil(FRAME_MS) as usize
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Silence,
    Stop,
    Cap,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Accepted speech; `samples` are 16 kHz mono f32 ready for inference.
    Utterance {
        samples: Vec<f32>,
        reason: FinishReason,
        speech_ms: u32,
    },
    /// No confirmed onset (timeout, stop or cap before speech).
    NoSpeech,
    /// Confirmed onset but speech span shorter than `min_speech_ms`.
    TooShort,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Continue,
    Onset,
    Finished(Outcome),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("endpointer already finished")]
pub struct AlreadyFinished;

pub struct Endpointer {
    cfg: EndpointConfig,
    preroll: VecDeque<[f32; FRAME_SAMPLES]>,
    window: VecDeque<bool>,
    /// Frames since onset, starting with the pre-roll frames.
    utterance: Vec<[f32; FRAME_SAMPLES]>,
    first_voiced: Option<usize>,
    last_voiced: Option<usize>,
    unvoiced_run: usize,
    total_frames: usize,
    onset: bool,
    done: bool,
}

impl Endpointer {
    pub fn new(cfg: EndpointConfig) -> Self {
        let max = frames(cfg.max_capture_ms);
        Self {
            cfg,
            preroll: VecDeque::with_capacity(
                frames(cfg.preroll_ms).max(cfg.onset_window as usize) + 1,
            ),
            window: VecDeque::with_capacity(cfg.onset_window as usize + 1),
            utterance: Vec::with_capacity(max.min(1024)),
            first_voiced: None,
            last_voiced: None,
            unvoiced_run: 0,
            total_frames: 0,
            onset: false,
            done: false,
        }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn push(
        &mut self,
        frame: &[f32; FRAME_SAMPLES],
        voiced: bool,
    ) -> Result<Event, AlreadyFinished> {
        if self.done {
            return Err(AlreadyFinished);
        }
        self.total_frames += 1;
        let ev = if self.onset {
            self.push_after_onset(frame, voiced)
        } else {
            self.push_before_onset(frame, voiced)
        };
        if !matches!(ev, Event::Finished(_)) && self.total_frames >= frames(self.cfg.max_capture_ms)
        {
            return Ok(self.finish(FinishReason::Cap));
        }
        Ok(ev)
    }

    /// Explicit stop: use available audio without waiting for silence.
    /// Idempotent with endpoint races: the first finish wins.
    pub fn stop(&mut self) -> Result<Outcome, AlreadyFinished> {
        if self.done {
            return Err(AlreadyFinished);
        }
        match self.finish(FinishReason::Stop) {
            Event::Finished(o) => Ok(o),
            _ => Ok(Outcome::NoSpeech),
        }
    }

    fn push_before_onset(&mut self, frame: &[f32; FRAME_SAMPLES], voiced: bool) -> Event {
        let keep = frames(self.cfg.preroll_ms).max(self.cfg.onset_window as usize);
        if self.preroll.len() == keep {
            self.preroll.pop_front();
        }
        self.preroll.push_back(*frame);
        if self.window.len() == self.cfg.onset_window as usize {
            self.window.pop_front();
        }
        self.window.push_back(voiced);
        let voiced_in_window = self.window.iter().filter(|v| **v).count() as u32;
        if voiced_in_window >= self.cfg.onset_voiced {
            self.onset = true;
            self.utterance.extend(self.preroll.drain(..));
            let n = self.utterance.len();
            let w = self.window.len();
            let first_in_window = self.window.iter().position(|v| *v).unwrap_or(0);
            self.first_voiced = Some(n - w + first_in_window);
            self.last_voiced = Some(n - 1);
            self.unvoiced_run = 0;
            return Event::Onset;
        }
        if self.total_frames >= frames(self.cfg.no_speech_timeout_ms) {
            self.done = true;
            self.preroll.clear();
            return Event::Finished(Outcome::NoSpeech);
        }
        Event::Continue
    }

    fn push_after_onset(&mut self, frame: &[f32; FRAME_SAMPLES], voiced: bool) -> Event {
        self.utterance.push(*frame);
        if voiced {
            self.last_voiced = Some(self.utterance.len() - 1);
            self.unvoiced_run = 0;
        } else {
            self.unvoiced_run += 1;
            if self.unvoiced_run >= frames(self.cfg.silence_ms) {
                return self.finish(FinishReason::Silence);
            }
        }
        Event::Continue
    }

    fn finish(&mut self, reason: FinishReason) -> Event {
        self.done = true;
        self.preroll.clear();
        let (Some(first), Some(last)) = (self.first_voiced, self.last_voiced) else {
            return Event::Finished(Outcome::NoSpeech);
        };
        if !self.onset {
            return Event::Finished(Outcome::NoSpeech);
        }
        let speech_frames = last - first + 1;
        let speech_ms = speech_frames as u32 * FRAME_MS;
        if speech_ms < self.cfg.min_speech_ms {
            self.utterance.clear();
            return Event::Finished(Outcome::TooShort);
        }
        let end = match reason {
            // Keep recorded trailing silence up to `trailing_ms`.
            FinishReason::Silence => {
                (last + 1 + frames(self.cfg.trailing_ms)).min(self.utterance.len())
            }
            FinishReason::Stop | FinishReason::Cap => self.utterance.len(),
        };
        let mut samples = Vec::with_capacity(end * FRAME_SAMPLES);
        for f in &self.utterance[..end] {
            samples.extend_from_slice(f);
        }
        self.utterance = Vec::new();
        Event::Finished(Outcome::Utterance {
            samples,
            reason,
            speech_ms,
        })
    }
}
