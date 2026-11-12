//! Stateful streaming resampler to 16 kHz mono (rubato FFT, fixed input chunk).
//! Accepts arbitrary block sizes; [`StreamResampler::flush`] emits the tail
//! exactly once with the startup delay trimmed, so total output length tracks
//! `input_len * 16000 / rate`.

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};

use crate::TARGET_RATE;

#[derive(Debug, thiserror::Error)]
pub enum ResampleError {
    #[error("unsupported input rate {0} Hz")]
    BadRate(u32),
    #[error("resampler: {0}")]
    Engine(String),
    #[error("flush already called")]
    Flushed,
}

pub struct StreamResampler {
    inner: Option<Fft<f32>>, // None when input is already 16 kHz
    chunk: usize,
    pending: Vec<f32>,
    out_buf: Vec<f32>,
    to_skip: usize,
    in_total: u64,
    out_total: u64,
    ratio: f64,
    flushed: bool,
}

impl StreamResampler {
    pub fn new(input_rate: u32) -> Result<Self, ResampleError> {
        if !(8_000..=384_000).contains(&input_rate) {
            return Err(ResampleError::BadRate(input_rate));
        }
        let ratio = f64::from(TARGET_RATE) / f64::from(input_rate);
        if input_rate == TARGET_RATE {
            return Ok(Self {
                inner: None,
                chunk: 0,
                pending: vec![],
                out_buf: vec![],
                to_skip: 0,
                in_total: 0,
                out_total: 0,
                ratio,
                flushed: false,
            });
        }
        // ~10 ms input chunks.
        let chunk = (input_rate / 100) as usize;
        let fft = Fft::<f32>::new(
            input_rate as usize,
            TARGET_RATE as usize,
            chunk,
            1,
            FixedSync::Input,
        )
        .map_err(|e| ResampleError::Engine(e.to_string()))?;
        let to_skip = fft.output_delay();
        let out_buf = vec![0.0; fft.output_frames_max()];
        Ok(Self {
            inner: Some(fft),
            chunk,
            pending: Vec::with_capacity(chunk * 2),
            out_buf,
            to_skip,
            in_total: 0,
            out_total: 0,
            ratio,
            flushed: false,
        })
    }

    /// Feed mono samples at the input rate; appends 16 kHz samples to `out`.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), ResampleError> {
        if self.flushed {
            return Err(ResampleError::Flushed);
        }
        self.in_total += input.len() as u64;
        if self.inner.is_none() {
            out.extend_from_slice(input);
            self.out_total += input.len() as u64;
            return Ok(());
        }
        self.pending.extend_from_slice(input);
        let mut start = 0;
        while self.pending.len() - start >= self.chunk {
            let end = start + self.chunk;
            self.process(start, end, None, out)?;
            start = end;
        }
        self.pending.drain(..start);
        Ok(())
    }

    /// Emit the remaining tail once. Output length is trimmed to the expected
    /// total so streaming and one-shot results agree.
    pub fn flush(&mut self, out: &mut Vec<f32>) -> Result<(), ResampleError> {
        if self.flushed {
            return Err(ResampleError::Flushed);
        }
        self.flushed = true;
        if self.inner.is_none() {
            return Ok(());
        }
        let expected = (self.in_total as f64 * self.ratio).round() as u64;
        // Feed the partial chunk, then silence until the delayed tail is out.
        let mut guard = 0;
        while self.out_total < expected && guard < 64 {
            let n = self.pending.len().min(self.chunk);
            self.pending.resize(self.chunk, 0.0);
            self.process(0, self.chunk, Some(n), out)?;
            self.pending.clear();
            guard += 1;
        }
        if self.out_total > expected {
            let excess = (self.out_total - expected) as usize;
            out.truncate(out.len().saturating_sub(excess));
            self.out_total = expected;
        }
        Ok(())
    }

    fn process(
        &mut self,
        start: usize,
        end: usize,
        partial: Option<usize>,
        out: &mut Vec<f32>,
    ) -> Result<(), ResampleError> {
        let Some(fft) = self.inner.as_mut() else {
            return Ok(());
        };
        let input = InterleavedSlice::new(&self.pending[start..end], 1, end - start)
            .map_err(|e| ResampleError::Engine(e.to_string()))?;
        let frames = self.out_buf.len();
        let mut output = InterleavedSlice::new_mut(&mut self.out_buf, 1, frames)
            .map_err(|e| ResampleError::Engine(e.to_string()))?;
        let indexing = partial.map(|p| Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len: Some(p),
            active_channels_mask: None,
        });
        let (_, produced) = fft
            .process_into_buffer(&input, &mut output, indexing.as_ref())
            .map_err(|e| ResampleError::Engine(e.to_string()))?;
        let skip = self.to_skip.min(produced);
        self.to_skip -= skip;
        out.extend_from_slice(&self.out_buf[skip..produced]);
        self.out_total += (produced - skip) as u64;
        Ok(())
    }
}
