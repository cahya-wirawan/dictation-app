//! Audio front end: native capture → finite f32 → mono → 16 kHz → 16 ms frames
//! → speech detection → endpointing. Everything except [`capture`] is
//! deterministic and testable without hardware.

pub mod capture;
pub mod convert;
pub mod detector;
pub mod dsp;
pub mod endpoint;
pub mod pipeline;
pub mod resample;

/// Whisper input rate.
pub const TARGET_RATE: u32 = 16_000;
/// Frame length required by the earshot VAD (16 ms at 16 kHz). See ADR-0001.
pub const FRAME_SAMPLES: usize = 256;
pub const FRAME_MS: u32 = 16;
