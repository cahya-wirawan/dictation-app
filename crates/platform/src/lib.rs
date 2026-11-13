//! Native target identity, permissions, clipboard and paste dispatch.
//!
//! Only the Rust coordinator may call the delivery functions; nothing here is
//! exposed over IPC. Snapshots hold native references and are deliberately not
//! `Serialize`, so a target token can never reach the frontend (docs/05 §4).
//!
//! Platform status: macOS (AX + CGEvent) and Windows (UI Automation +
//! SendInput) adapters; every other platform reports `Unknown`, which
//! disables automatic delivery and leaves the draft/Copy path.

use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use win32::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod unsupported;
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub use unsupported::*;

#[derive(Debug, Clone, PartialEq)]
pub struct AppIdentity {
    pub pid: i32,
    pub bundle_id: Option<String>,
    /// Process launch time; distinguishes PID reuse.
    pub launch_time: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldClass {
    Editable,
    /// Password/secure field: automatic delivery is never allowed.
    Secure,
    NotEditable,
    /// No element identity available. Unknown is *not* evidence of safety.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetCheck {
    Same,
    AppChanged,
    WindowChanged,
    FieldChanged,
    OwnApp,
    Secure,
    NotEditable,
    Unknown,
}

impl TargetCheck {
    /// Only an unchanged, editable, non-secure, non-self target qualifies.
    pub fn allows_auto_delivery(self) -> bool {
        self == TargetCheck::Same
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Granted,
    Denied,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionKind {
    Microphone,
    /// macOS: required for target validation and the paste chord.
    Accessibility,
}

/// OS permission state as reported by the platform, never a saved grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    Granted,
    Denied,
    NotDetermined,
    /// Blocked by policy (MDM, parental controls); the user cannot change it.
    Restricted,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// The OS API accepted all synthetic events. This does *not* prove the
    /// destination received text (docs/05 §5 step 8).
    Sent,
    /// Partial or failed submission; never retry automatically.
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    #[error("accessibility permission not granted")]
    NotTrusted,
    #[error("no frontmost application")]
    NoFrontmostApp,
    #[error("clipboard write failed")]
    ClipboardFailed,
    #[error("not supported on this platform")]
    Unsupported,
}

/// Wait until the user has physically released all modifiers, polling the OS
/// state. Never synthesizes releases. Returns false on timeout or when the
/// platform cannot observe modifier state.
pub fn wait_modifiers_released(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match physical_modifiers_down() {
            Some(false) => return true,
            None => return false,
            Some(true) if Instant::now() >= deadline => return false,
            Some(true) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Default modifier-release budget (docs/05 §4 step 5).
pub const MODIFIER_RELEASE_TIMEOUT: Duration = Duration::from_millis(1500);
