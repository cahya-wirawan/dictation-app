//! Side-effecting half of the core: runs the pure session reducer from
//! `dictation_domain::session` as a single-threaded actor and executes its
//! effects through adapter traits. Lives outside `src-tauri` so it can be
//! tested without a webview (docs/07 §5).

pub mod actor;
pub mod adapters;
pub mod config_store;
