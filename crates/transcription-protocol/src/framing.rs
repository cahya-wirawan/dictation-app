use std::io::{self, Read, Write};

use serde::{Serialize, de::DeserializeOwned};

use crate::{MAX_ENVELOPE_BYTES, MAX_SAMPLES};

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    #[error("peer closed the pipe mid-frame")]
    Truncated,
    #[error("frame length {0} outside 1..={MAX_ENVELOPE_BYTES}")]
    BadLength(u32),
    #[error("envelope is not valid protocol JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("message failed validation: {0}")]
    Invalid(String),
    #[error("PCM contains non-finite samples")]
    NonFinitePcm,
}

/// Writes one length-prefixed JSON envelope and flushes.
pub fn write_frame<W: Write, T: Serialize>(w: &mut W, msg: &T) -> Result<(), FrameError> {
    let body = serde_json::to_vec(msg)?;
    let len = u32::try_from(body.len()).map_err(|_| FrameError::BadLength(u32::MAX))?;
    if len == 0 || len > MAX_ENVELOPE_BYTES {
        return Err(FrameError::BadLength(len));
    }
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&body)?;
    w.flush()?;
    Ok(())
}

/// Reads one envelope. `Ok(None)` means clean EOF *between* frames, which is
/// the parent-liveness signal for the worker. The length is checked before
/// any allocation.
pub fn read_frame<R: Read, T: DeserializeOwned>(r: &mut R) -> Result<Option<T>, FrameError> {
    let mut len_buf = [0u8; 4];
    match read_exact_or_eof(r, &mut len_buf)? {
        ReadOutcome::Eof => return Ok(None),
        ReadOutcome::Partial => return Err(FrameError::Truncated),
        ReadOutcome::Full => {}
    }
    let len = u32::from_le_bytes(len_buf);
    if len == 0 || len > MAX_ENVELOPE_BYTES {
        return Err(FrameError::BadLength(len));
    }
    let mut body = vec![0u8; len as usize];
    if !matches!(read_exact_or_eof(r, &mut body)?, ReadOutcome::Full) {
        return Err(FrameError::Truncated);
    }
    Ok(Some(serde_json::from_slice(&body)?))
}

/// Writes PCM as LE f32. Caller has already validated finiteness and length.
pub fn write_pcm<W: Write>(w: &mut W, pcm: &[f32]) -> Result<(), FrameError> {
    if pcm.len() > MAX_SAMPLES as usize {
        return Err(FrameError::Invalid("too many samples".into()));
    }
    // Chunked to avoid a second full-size allocation.
    let mut buf = Vec::with_capacity(16 * 1024);
    for chunk in pcm.chunks(4096) {
        buf.clear();
        for s in chunk {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        w.write_all(&buf)?;
    }
    w.flush()?;
    Ok(())
}

/// Reads exactly `sample_count` LE f32 samples, rejecting non-finite values.
pub fn read_pcm<R: Read>(r: &mut R, sample_count: u32) -> Result<Vec<f32>, FrameError> {
    if sample_count == 0 || sample_count > MAX_SAMPLES {
        return Err(FrameError::Invalid("sample_count out of range".into()));
    }
    let mut out = Vec::with_capacity(sample_count as usize);
    let mut buf = vec![0u8; 16 * 1024];
    let mut remaining = sample_count as usize * 4;
    while remaining > 0 {
        let n = remaining.min(buf.len());
        if !matches!(read_exact_or_eof(r, &mut buf[..n])?, ReadOutcome::Full) {
            return Err(FrameError::Truncated);
        }
        for b in buf[..n].chunks_exact(4) {
            let s = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            if !s.is_finite() {
                return Err(FrameError::NonFinitePcm);
            }
            out.push(s);
        }
        remaining -= n;
    }
    Ok(out)
}

pub fn has_non_finite(pcm: &[f32]) -> bool {
    pcm.iter().any(|s| !s.is_finite())
}

enum ReadOutcome {
    Full,
    Eof,
    Partial,
}

fn read_exact_or_eof<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<ReadOutcome> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => {
                return Ok(if filled == 0 {
                    ReadOutcome::Eof
                } else {
                    ReadOutcome::Partial
                });
            }
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(ReadOutcome::Full)
}
