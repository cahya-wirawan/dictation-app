//! Fallback for platforms without a qualified adapter: no target identity,
//! therefore no automatic delivery. Capture/transcription/Copy still work.

use crate::*;

#[derive(Debug)]
pub struct TargetSnapshot {
    pub app: Option<AppIdentity>,
    pub field: FieldClass,
}

pub fn accessibility_permission() -> Permission {
    Permission::NotApplicable
}

pub fn capture_target() -> Result<TargetSnapshot, PlatformError> {
    Ok(TargetSnapshot {
        app: None,
        field: FieldClass::Unknown,
    })
}

pub fn check_target(_start: &TargetSnapshot) -> TargetCheck {
    TargetCheck::Unknown
}

pub fn physical_modifiers_down() -> Option<bool> {
    None
}

pub fn clipboard_write_text(_text: &str) -> Result<i64, PlatformError> {
    Err(PlatformError::Unsupported)
}

pub fn send_paste_chord() -> Result<DispatchOutcome, PlatformError> {
    Err(PlatformError::Unsupported)
}

pub fn focused_text_value() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_target_never_auto_delivers() {
        let s = capture_target().unwrap();
        assert_eq!(s.field, FieldClass::Unknown);
        assert!(!check_target(&s).allows_auto_delivery());
        assert!(!wait_modifiers_released(std::time::Duration::from_millis(
            1
        )));
        assert!(send_paste_chord().is_err());
    }
}
