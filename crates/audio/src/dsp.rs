//! DSP worker (docs/03 §3, work package A2): one thread per capture session
//! that drains the capture ring into the [`Pipeline`] and reports events.
//!
//! Ordering guarantee: the source (microphone) is closed *before* any terminal
//! event (`Finished`, `Fault`, `Aborted`) is emitted, so inference never runs
//! with the microphone open and a fault never yields a silently truncated
//! utterance. The exception is continuous dictation (ADR-014): there a
//! [`DspEvent::Segment`] reports each finished utterance while the microphone
//! stays open for the next one; the terminal events keep the guarantee.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::capture::{CaptureError, CaptureHandle, CaptureReader, NativeFormat};
use crate::detector::SpeechDetector;
use crate::endpoint::{EndpointConfig, Outcome};
use crate::pipeline::{Pipeline, PipelineError};

/// Meter events are rate-limited to ≤15 Hz (docs/03 §3).
pub const METER_INTERVAL: Duration = Duration::from_millis(67);

/// Anything that yields interleaved native-rate samples and can be closed.
pub trait AudioSource: Send {
    fn format(&self) -> NativeFormat;
    fn drain_into(&mut self, out: &mut Vec<f32>) -> Result<usize, CaptureError>;
    /// Close the device. Must be idempotent.
    fn close(&mut self);
}

/// Real microphone: a CPAL stream handle plus its ring consumer.
pub struct LiveCapture {
    pub handle: CaptureHandle,
    pub reader: CaptureReader,
}

impl AudioSource for LiveCapture {
    fn format(&self) -> NativeFormat {
        self.reader.format
    }
    fn drain_into(&mut self, out: &mut Vec<f32>) -> Result<usize, CaptureError> {
        self.reader.drain_into(out)
    }
    fn close(&mut self) {
        self.handle.stop();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DspEvent {
    /// Normalized level in [0, 1]; emitted at most every [`METER_INTERVAL`].
    Level(f32),
    /// Confirmed speech onset (emitted once).
    Onset,
    /// Continuous mode only: one utterance ended; the microphone stays open.
    /// Never `NoSpeech` (the no-speech timeout is terminal).
    Segment(Outcome),
    /// Terminal: endpoint, cap or explicit stop. Microphone already closed.
    Finished(Outcome),
    /// Terminal: device/format/overflow failure. Microphone already closed.
    Fault(DspFault),
    /// Terminal: aborted on request. Microphone already closed.
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DspFault {
    Capture(CaptureError),
    InvalidAudio,
}

enum Control {
    Stop,
    Abort,
}

/// Handle to a running DSP thread. Dropping it aborts the session.
pub struct DspSession {
    tx: Sender<Control>,
    join: Option<JoinHandle<()>>,
}

impl DspSession {
    pub fn spawn<F>(
        source: Box<dyn AudioSource>,
        detector: Box<dyn SpeechDetector>,
        cfg: EndpointConfig,
        poll: Duration,
        sink: F,
    ) -> Result<Self, DspFault>
    where
        F: FnMut(DspEvent) + Send + 'static,
    {
        Self::spawn_with(source, detector, cfg, poll, false, sink)
    }

    /// Like [`DspSession::spawn`]; `segmenting` selects continuous dictation.
    pub fn spawn_with<F>(
        mut source: Box<dyn AudioSource>,
        detector: Box<dyn SpeechDetector>,
        cfg: EndpointConfig,
        poll: Duration,
        segmenting: bool,
        mut sink: F,
    ) -> Result<Self, DspFault>
    where
        F: FnMut(DspEvent) + Send + 'static,
    {
        let f = source.format();
        let pipeline = Pipeline::new(f.sample_rate, f.channels, detector, cfg)
            .map(|p| if segmenting { p.segmenting() } else { p });
        let mut pipeline = match pipeline {
            Ok(p) => p,
            Err(_) => {
                source.close();
                return Err(DspFault::InvalidAudio);
            }
        };
        let (tx, rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name("dsp".into())
            .spawn(move || run(source.as_mut(), &mut pipeline, &rx, poll, &mut sink))
            .map_err(|_| DspFault::InvalidAudio)?;
        Ok(Self {
            tx,
            join: Some(join),
        })
    }

    /// Explicit stop: finalize with the audio captured so far. Idempotent.
    pub fn stop(&self) {
        let _ = self.tx.send(Control::Stop);
    }

    /// Discard audio and close the microphone. Idempotent.
    pub fn abort(&self) {
        let _ = self.tx.send(Control::Abort);
    }

    /// Wait for the thread to finish (after a terminal event).
    pub fn join(mut self) {
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for DspSession {
    fn drop(&mut self) {
        let _ = self.tx.send(Control::Abort);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn run(
    source: &mut dyn AudioSource,
    pipeline: &mut Pipeline,
    rx: &Receiver<Control>,
    poll: Duration,
    sink: &mut dyn FnMut(DspEvent),
) {
    let mut buf = Vec::with_capacity(8192);
    let mut last_meter: Option<Instant> = None;
    let mut onset_sent = false;
    loop {
        let control = match rx.try_recv() {
            Ok(c) => Some(c),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Control::Abort),
        };
        if let Some(Control::Abort) = control {
            source.close();
            sink(DspEvent::Aborted);
            return;
        }

        buf.clear();
        if let Err(e) = source.drain_into(&mut buf) {
            source.close();
            sink(DspEvent::Fault(DspFault::Capture(e)));
            return;
        }
        let pushed = if buf.is_empty() {
            Ok(None)
        } else {
            pipeline.push(&buf)
        };
        for seg in pipeline.take_segments() {
            sink(DspEvent::Segment(seg));
        }
        match pushed {
            Ok(Some(outcome)) => {
                source.close();
                sink(DspEvent::Finished(outcome));
                return;
            }
            Ok(None) => {}
            Err(PipelineError::Finished(_)) => {
                source.close();
                sink(DspEvent::Aborted);
                return;
            }
            Err(_) => {
                source.close();
                sink(DspEvent::Fault(DspFault::InvalidAudio));
                return;
            }
        }
        if !onset_sent && pipeline.has_onset() {
            onset_sent = true;
            sink(DspEvent::Onset);
        }
        if !buf.is_empty() && last_meter.is_none_or(|t| t.elapsed() >= METER_INTERVAL) {
            sink(DspEvent::Level(pipeline.level()));
            last_meter = Some(Instant::now());
        }

        if let Some(Control::Stop) = control {
            // Take whatever is already in the ring, then close and finalize.
            buf.clear();
            let tail = source.drain_into(&mut buf);
            source.close();
            if tail.is_err() {
                sink(DspEvent::Fault(DspFault::InvalidAudio));
                return;
            }
            let pushed = if buf.is_empty() {
                Ok(None)
            } else {
                pipeline.push(&buf)
            };
            for seg in pipeline.take_segments() {
                sink(DspEvent::Segment(seg));
            }
            if let Ok(Some(outcome)) = pushed {
                sink(DspEvent::Finished(outcome));
                return;
            }
            let last = pipeline.stop();
            for seg in pipeline.take_segments() {
                sink(DspEvent::Segment(seg));
            }
            match last {
                Ok(o) => sink(DspEvent::Finished(o)),
                Err(_) => sink(DspEvent::Fault(DspFault::InvalidAudio)),
            }
            return;
        }
        thread::sleep(poll);
    }
}
