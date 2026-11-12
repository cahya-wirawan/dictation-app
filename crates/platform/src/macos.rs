//! macOS adapter: NSWorkspace for app identity, Accessibility (AX) for window
//! and focused-element identity, NSPasteboard for text, CGEvent for one Cmd+V.
//!
//! UNVERIFIED (M0): compiled for aarch64-apple-darwin only; not yet run on a
//! Mac. Behavior must be confirmed with the M0 probes before it is trusted.

use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSRunningApplication, NSWorkspace};
use objc2_application_services::{
    AXError, AXIsProcessTrusted, AXIsProcessTrustedWithOptions, AXUIElement,
    kAXTrustedCheckOptionPrompt,
};
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFEqual, CFRetained, CFString, CFType};
use objc2_core_graphics::{
    CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
};
use objc2_foundation::{NSString, NSURL};

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

pub fn permission_status(kind: PermissionKind) -> PermissionStatus {
    match kind {
        PermissionKind::Accessibility => {
            if accessibility_permission() == Permission::Granted {
                PermissionStatus::Granted
            } else {
                // AX has no "not determined": untrusted until the user enables it.
                PermissionStatus::Denied
            }
        }
        PermissionKind::Microphone => {
            // SAFETY: static AVFoundation constant; class method without side effects.
            let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
                return PermissionStatus::NotApplicable;
            };
            match unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) } {
                AVAuthorizationStatus::Authorized => PermissionStatus::Granted,
                AVAuthorizationStatus::Denied => PermissionStatus::Denied,
                AVAuthorizationStatus::Restricted => PermissionStatus::Restricted,
                _ => PermissionStatus::NotDetermined,
            }
        }
    }
}

/// User-initiated request (docs/05 §2): the microphone shows the system
/// prompt once; Accessibility shows the "open System Settings" prompt. The
/// callback gets the status afterwards (Accessibility is granted in System
/// Settings, so callers must re-check later).
pub fn request_permission(kind: PermissionKind, done: Box<dyn FnOnce(PermissionStatus) + Send>) {
    match kind {
        PermissionKind::Accessibility => {
            let key: &CFString = unsafe { kAXTrustedCheckOptionPrompt };
            let value = CFBoolean::new(true);
            let opts = CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[value]);
            // SAFETY: dictionary with the documented key/value types.
            let trusted = unsafe { AXIsProcessTrustedWithOptions(Some(opts.as_ref())) };
            done(if trusted {
                PermissionStatus::Granted
            } else {
                PermissionStatus::Denied
            });
        }
        PermissionKind::Microphone => {
            let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
                return done(PermissionStatus::NotApplicable);
            };
            if permission_status(kind) != PermissionStatus::NotDetermined {
                return done(permission_status(kind));
            }
            let slot = std::sync::Mutex::new(Some(done));
            let block = block2::RcBlock::new(move |granted: objc2::runtime::Bool| {
                if let Some(f) = slot.lock().ok().and_then(|mut s| s.take()) {
                    f(if granted.as_bool() {
                        PermissionStatus::Granted
                    } else {
                        PermissionStatus::Denied
                    });
                }
            });
            // SAFETY: valid media type and a block that outlives the call (retained by AVFoundation).
            unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &block) };
        }
    }
}

/// Opens the matching Privacy & Security pane. Returns false if it failed.
pub fn open_privacy_settings(kind: PermissionKind) -> bool {
    let pane = match kind {
        PermissionKind::Microphone => "Privacy_Microphone",
        PermissionKind::Accessibility => "Privacy_Accessibility",
    };
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{pane}");
    let Some(url) = NSURL::URLWithString(&NSString::from_str(&url)) else {
        return false;
    };
    NSWorkspace::sharedWorkspace().openURL(&url)
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

/// Observers that turn sleep, display sleep, screen lock, fast user switching
/// and logout/power-off into one callback (docs/04 invariant 10: cancel, clear
/// the draft, never resume a paste on wake). Dropping it removes them.
pub struct SuspendWatch {
    workspace: Vec<Retained<objc2::runtime::ProtocolObject<dyn objc2::runtime::NSObjectProtocol>>>,
    distributed:
        Vec<Retained<objc2::runtime::ProtocolObject<dyn objc2::runtime::NSObjectProtocol>>>,
}

/// Must be called on the main thread (NSWorkspace notifications are posted
/// there); `on_suspend` must only send a message, never block.
pub fn watch_suspend(on_suspend: std::sync::Arc<dyn Fn() + Send + Sync>) -> SuspendWatch {
    use objc2_app_kit::{
        NSWorkspaceScreensDidSleepNotification, NSWorkspaceSessionDidResignActiveNotification,
        NSWorkspaceWillPowerOffNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::{NSDistributedNotificationCenter, NSNotification};

    let block = {
        let cb = std::sync::Arc::clone(&on_suspend);
        block2::RcBlock::new(move |_n: NonNull<NSNotification>| cb())
    };
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: static AppKit notification names.
    let names = unsafe {
        [
            NSWorkspaceWillSleepNotification,
            NSWorkspaceScreensDidSleepNotification,
            NSWorkspaceSessionDidResignActiveNotification,
            NSWorkspaceWillPowerOffNotification,
        ]
    };
    let workspace = names
        .into_iter()
        .map(|name| {
            // SAFETY: nil object/queue (deliver on the posting thread); the block
            // only calls a Send + Sync closure.
            unsafe {
                center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
            }
        })
        .collect();
    // Screen lock is only announced as a distributed notification.
    let dist = NSDistributedNotificationCenter::defaultCenter();
    let locked = NSString::from_str("com.apple.screenIsLocked");
    // SAFETY: as above.
    let distributed = vec![unsafe {
        dist.addObserverForName_object_queue_usingBlock(Some(&locked), None, None, &block)
    }];
    SuspendWatch {
        workspace,
        distributed,
    }
}

impl Drop for SuspendWatch {
    fn drop(&mut self) {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        for o in &self.workspace {
            // SAFETY: tokens returned by addObserverForName on this center.
            unsafe { center.removeObserver(o.as_ref()) };
        }
        let dist = objc2_foundation::NSDistributedNotificationCenter::defaultCenter();
        for o in &self.distributed {
            // SAFETY: as above.
            unsafe { dist.removeObserver(o.as_ref()) };
        }
    }
}
