//! Model manager (docs/03 §6, work package B4).
//!
//! The manifest compiled into the signed app is the only source of model
//! URLs, sizes and hashes; the frontend can only name a manifest id. Files are
//! downloaded or imported into a private `.partial`, hashed while streaming,
//! fsynced and atomically renamed. Only verified files ever count as
//! installed, and `.partial` files never do.

pub mod download;
pub mod manifest;
pub mod store;

pub use manifest::{APPROVED, ApprovedModel, find};
pub use store::{Cancel, ModelError, ModelState, ModelStatus, ModelStore, Progress};
