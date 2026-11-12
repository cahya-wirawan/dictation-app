//! Wire protocol v1 between the app (parent) and the bundled `dictation-worker`.
//!
//! Transport: the worker's inherited stdin/stdout anonymous pipes. Every message
//! is a 4-byte little-endian length followed by a UTF-8 JSON envelope of at most
//! [`MAX_ENVELOPE_BYTES`]. A `Transcribe` envelope is immediately followed by
//! exactly `sample_count * 4` bytes of little-endian f32 PCM, never interleaved
//! with other frames. See docs/04 §5.

pub mod client;
pub mod framing;
mod job;
pub mod messages;

pub use framing::{FrameError, read_frame, read_pcm, write_frame, write_pcm};
pub use messages::*;

pub const PROTOCOL_VERSION: u32 = 1;
/// Maximum JSON envelope size (256 KiB).
pub const MAX_ENVELOPE_BYTES: u32 = 256 * 1024;
/// 16 kHz mono.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
/// 120 s at 16 kHz.
pub const MAX_SAMPLES: u32 = 1_920_000;
/// Text limit before JSON escaping (32 KiB).
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
/// Upper bound on decoder segments reported per job.
pub const MAX_SEGMENTS: u32 = 512;
