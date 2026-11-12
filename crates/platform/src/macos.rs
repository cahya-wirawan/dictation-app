//! macOS adapter: NSWorkspace for app identity, Accessibility (AX) for window
//! and focused-element identity, NSPasteboard for text, CGEvent for one Cmd+V.
//!
//! UNVERIFIED (M0): compiled for aarch64-apple-darwin only; not yet run on a
//! Mac. Behavior must be confirmed with the M0 probes before it is trusted.

use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSRunningApplication, NSWorkspace};
use objc2_application_services::{AXError, AXIsProcessTrusted, AXUIElement};
use objc2_core_foundation::{CFEqual, CFRetained, CFString, CFType};
use objc2_core_graphics::{
    CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
};
use objc2_foundation::NSString;

use crate::*;

/// kVK_ANSI_V. Layout-independent virtual key code for the physical V key.
const KEY_V: u16 = 9;

pub struct TargetSnapshot {
    pub app: AppIdentity,
    running: Retained<NSRunningApplication>,
    pub field: FieldClass,
    window: Option<CFRetained<CFType>>,
    element: Option<CFRetained<CFType>>,
}

impl std::fmt::Debug for TargetSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TargetSnapshot")
            .field("app", &self.app)
            .field("field", &self.field)
            .field("has_window", &self.window.is_some())
            .field("has_element", &self.element.is_some())
            .finish()
    }
}

pub fn accessibility_permission() -> Permission {
    // SAFETY: no arguments; reads TCC state for this process.
    if unsafe { AXIsProcessTrusted() } {
        Permission::Granted
    } else {
        Permission::Denied
    }
}

fn frontmost_app() -> Option<(AppIdentity, Retained<NSRunningApplication>)> {
    let ws = NSWorkspace::sharedWorkspace();
    let app = ws.frontmostApplication()?;
    let id = AppIdentity {
        pid: app.processIdentifier(),
        bundle_id: app.bundleIdentifier().map(|s| s.to_string()),
        launch_time: app.launchDate().map(|d| d.timeIntervalSinceReferenceDate()),
    };
    Some((id, app))
}

fn copy_attr(element: &AXUIElement, name: &'static str) -> Option<CFRetained<CFType>> {
    let attr = CFString::from_static_str(name);
    let mut out: *const CFType = std::ptr::null();
    // SAFETY: `out` is a valid out-pointer; on success it holds a +1 reference.
    let err = unsafe { element.copy_attribute_value(&attr, NonNull::from(&mut out)) };
    if err != AXError::Success {
        return None;
    }
    // SAFETY: Copy rule: we own the returned reference.
    NonNull::new(out.cast_mut()).map(|p| unsafe { CFRetained::from_raw(p) })
}

fn as_element(v: &CFType) -> Option<&AXUIElement> {
    v.downcast_ref::<AXUIElement>()
}

fn string_attr(element: &AXUIElement, name: &'static str) -> Option<String> {
    let v = copy_attr(element, name)?;
    v.downcast_ref::<CFString>().map(|s| s.to_string())
}

fn classify(element: &AXUIElement) -> FieldClass {
    let role = string_attr(element, "AXRole");
    let subrole = string_attr(element, "AXSubrole");
    if subrole.as_deref() == Some("AXSecureTextField") {
        return FieldClass::Secure;
    }
    let attr = CFString::from_static_str("AXValue");
    let mut settable: u8 = 0;
    // SAFETY: valid out-pointer to a Boolean.
    let err = unsafe { element.is_attribute_settable(&attr, NonNull::from(&mut settable)) };
    match role.as_deref() {
        Some("AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField") => FieldClass::Editable,
        Some(_) if err == AXError::Success && settable != 0 => FieldClass::Editable,
        Some(_) => FieldClass::NotEditable,
        None => FieldClass::Unknown,
    }
}

pub fn capture_target() -> Result<TargetSnapshot, PlatformError> {
    if accessibility_permission() != Permission::Granted {
        return Err(PlatformError::NotTrusted);
    }
    let (app, running) = frontmost_app().ok_or(PlatformError::NoFrontmostApp)?;
    // SAFETY: creating AX references is side-effect free.
    let app_el = unsafe { AXUIElement::new_application(app.pid) };
    let window = copy_attr(&app_el, "AXFocusedWindow");
    // SAFETY: as above.
    let system = unsafe { AXUIElement::new_system_wide() };
    let element = copy_attr(&system, "AXFocusedUIElement");
    let field = element
        .as_deref()
        .and_then(as_element)
        .map(classify)
        .unwrap_or(FieldClass::Unknown);
    Ok(TargetSnapshot {
        app,
        running,
        field,
        window,
        element,
    })
}

fn same_ref(a: &Option<CFRetained<CFType>>, b: &Option<CFRetained<CFType>>) -> Option<bool> {
    match (a, b) {
        (Some(a), Some(b)) => Some(CFEqual(Some(a), Some(b))),
        _ => None, // missing identity is unknown, not "same"
    }
}

/// Re-validates the start token (docs/05 §4 steps 2–4 and 6).
pub fn check_target(start: &TargetSnapshot) -> TargetCheck {
    if start.app.pid == std::process::id() as i32 {
        return TargetCheck::OwnApp;
    }
    let Ok(now) = capture_target() else {
        return TargetCheck::Unknown;
    };
    // Apple: compare NSRunningApplication with -isEqual:, not by pid.
    if now.app != start.app || now.running != start.running {
        return TargetCheck::AppChanged;
    }
    match same_ref(&start.window, &now.window) {
        Some(false) => return TargetCheck::WindowChanged,
        None => return TargetCheck::Unknown,
        Some(true) => {}
    }
    match same_ref(&start.element, &now.element) {
        Some(false) => return TargetCheck::FieldChanged,
        None => return TargetCheck::Unknown,
        Some(true) => {}
    }
    match now.field {
        FieldClass::Editable => TargetCheck::Same,
        FieldClass::Secure => TargetCheck::Secure,
        FieldClass::NotEditable => TargetCheck::NotEditable,
        FieldClass::Unknown => TargetCheck::Unknown,
    }
}

pub fn physical_modifiers_down() -> Option<bool> {
    let flags = CGEventSource::flags_state(CGEventSourceStateID::HIDSystemState);
    let mask = CGEventFlags::MaskCommand
        | CGEventFlags::MaskControl
        | CGEventFlags::MaskAlternate
        | CGEventFlags::MaskShift;
    Some(flags.intersects(mask))
}

/// Writes plain Unicode text only. Returns the new change count, which acts as
/// the ownership/revision token for any later restore logic.
pub fn clipboard_write_text(text: &str) -> Result<i64, PlatformError> {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    let s = NSString::from_str(text);
    // SAFETY: NSPasteboardTypeString is a static AppKit constant.
    let ok = pb.setString_forType(&s, unsafe { NSPasteboardTypeString });
    if !ok {
        return Err(PlatformError::ClipboardFailed);
    }
    Ok(pb.changeCount() as i64)
}

/// Posts exactly one Cmd+V (key down + key up). The Command flag is carried on
/// the V events themselves, so no modifier key is ever synthesized and nothing
/// the user holds is released. Key-up is always attempted.
pub fn send_paste_chord() -> Result<DispatchOutcome, PlatformError> {
    if accessibility_permission() != Permission::Granted {
        return Err(PlatformError::NotTrusted);
    }
    let down = CGEvent::new_keyboard_event(None, KEY_V, true);
    let up = CGEvent::new_keyboard_event(None, KEY_V, false);
    let (Some(down), Some(up)) = (down, up) else {
        return Ok(DispatchOutcome::Uncertain);
    };
    CGEvent::set_flags(Some(&down), CGEventFlags::MaskCommand);
    CGEvent::set_flags(Some(&up), CGEventFlags::MaskCommand);
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&down));
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&up));
    // CGEventPost returns void: "Sent" means only that both events were posted.
    Ok(DispatchOutcome::Sent)
}

/// Diagnostics only: the focused element's AXValue as text, used by the M0
/// probe to inspect what the receiving app actually contains.
pub fn focused_text_value() -> Option<String> {
    // SAFETY: side-effect free AX reads.
    let system = unsafe { AXUIElement::new_system_wide() };
    let el = copy_attr(&system, "AXFocusedUIElement")?;
    string_attr(as_element(&el)?, "AXValue")
}
