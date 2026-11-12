//! Sample-format conversion and channel downmix.

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConvertError {
    #[error("non-finite sample in input")]
    NonFinite,
    #[error("channel count must be >= 1")]
    NoChannels,
    #[error("buffer length is not a multiple of the channel count")]
    Ragged,
}

pub fn i16_to_f32(s: i16) -> f32 {
    f32::from(s) / 32768.0
}

pub fn u16_to_f32(s: u16) -> f32 {
    (f32::from(s) - 32768.0) / 32768.0
}

/// Downmix interleaved frames to mono with equal weights (arithmetic mean),
/// appending to `out`. Rejects NaN/inf instead of passing them downstream and
/// clamps to [-1, 1].
pub fn downmix_into(
    interleaved: &[f32],
    channels: usize,
    out: &mut Vec<f32>,
) -> Result<(), ConvertError> {
    if channels == 0 {
        return Err(ConvertError::NoChannels);
    }
    if !interleaved.len().is_multiple_of(channels) {
        return Err(ConvertError::Ragged);
    }
    let scale = 1.0 / channels as f32;
    for frame in interleaved.chunks_exact(channels) {
        let mut acc = 0.0f32;
        for &s in frame {
            if !s.is_finite() {
                return Err(ConvertError::NonFinite);
            }
            acc += s;
        }
        out.push((acc * scale).clamp(-1.0, 1.0));
    }
    Ok(())
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

pub fn dbfs(rms: f32) -> f32 {
    20.0 * rms.max(1e-10).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_formats_map_to_unit_range() {
        assert_eq!(i16_to_f32(i16::MIN), -1.0);
        assert!((i16_to_f32(i16::MAX) - 1.0).abs() < 1e-4);
        assert_eq!(u16_to_f32(32768), 0.0);
        assert_eq!(u16_to_f32(0), -1.0);
    }

    #[test]
    fn stereo_downmix_is_mean() {
        let mut out = vec![];
        downmix_into(&[1.0, 0.0, -0.5, -0.5, 0.2, 0.4], 2, &mut out).unwrap();
        assert_eq!(out.len(), 3);
        assert!(
            (out[0] - 0.5).abs() < 1e-6
                && (out[1] + 0.5).abs() < 1e-6
                && (out[2] - 0.3).abs() < 1e-6
        );
    }

    #[test]
    fn rejects_nan_and_ragged() {
        let mut out = vec![];
        assert_eq!(
            downmix_into(&[0.0, f32::NAN], 2, &mut out),
            Err(ConvertError::NonFinite)
        );
        assert_eq!(
            downmix_into(&[0.0, 0.0, 0.0], 2, &mut out),
            Err(ConvertError::Ragged)
        );
        assert_eq!(
            downmix_into(&[0.0], 0, &mut out),
            Err(ConvertError::NoChannels)
        );
    }
}
