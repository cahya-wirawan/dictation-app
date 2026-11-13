//! Linux (M7, experimental; docs/05 §1). What is implemented and why:
//!
//! * **No automatic paste.** Neither X11 nor Wayland gives this app a
//!   focused *editable element* identity without AT-SPI, which is not
//!   qualified. docs/05 §4 then requires explicit draft delivery, so
//!   `check_target` never returns `Same` and `send_paste_chord` is
//!   unsupported.
//! * **Window-level identity on X11** (EWMH `_NET_ACTIVE_WINDOW`,
//!   `_NET_WM_PID`, `WM_CLASS`) is still captured, for the probe and so a
//!   changed window is reported accurately.
//! * **Copy works on X11 and on Wayland sessions with XWayland.** The app
//!   owns the X11 `CLIPBOARD` selection from a small server thread until
//!   another client takes it. Pure Wayland without XWayland has no Copy.
//! * **Sleep, lock and shutdown** come from systemd-logind over D-Bus.
//! * [`probe`] reports the session so the UI can explain what works.

use std::sync::mpsc;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
    SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

use crate::*;

/// Clipboard text is bounded by the draft limit (32 KiB), far below the X11
/// maximum request size, so the INCR protocol is never needed.
const MAX_CLIPBOARD_BYTES: usize = 64 * 1024;
const CLIPBOARD_BUDGET: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    X11,
    Wayland,
    /// No graphical session variables (tty, ssh, CI without a display).
    None,
}

/// What the current Linux session supports, measured rather than assumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxSession {
    pub kind: SessionKind,
    /// `XDG_CURRENT_DESKTOP`, e.g. `GNOME`, `KDE`, `XFCE`.
    pub desktop: Option<String>,
    /// An X server is reachable (native X11 or XWayland).
    pub x11_reachable: bool,
    /// An EWMH window manager publishes `_NET_ACTIVE_WINDOW`.
    pub ewmh_active_window: bool,
    /// Global shortcuts through the X11 key grab used by the shortcut plugin.
    /// On Wayland this only sees keys while an XWayland window is focused.
    pub global_shortcuts_reliable: bool,
    /// The xdg-desktop-portal GlobalShortcuts interface is present on the
    /// session bus. Not used yet; reported for the support matrix.
    pub portal_global_shortcuts: bool,
    pub logind: bool,
}

pub fn probe() -> LinuxSession {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let kind = match env("XDG_SESSION_TYPE").as_deref() {
        Some("wayland") => SessionKind::Wayland,
        Some("x11") => SessionKind::X11,
        _ if env("WAYLAND_DISPLAY").is_some() => SessionKind::Wayland,
        _ if env("DISPLAY").is_some() => SessionKind::X11,
        _ => SessionKind::None,
    };
    let x = env("DISPLAY").and_then(|_| x11rb::connect(None).ok());
    let ewmh_active_window = x
        .as_ref()
        .is_some_and(|(c, s)| active_window(c, c.setup().roots[*s].root).is_some());
    LinuxSession {
        kind,
        desktop: env("XDG_CURRENT_DESKTOP"),
        x11_reachable: x.is_some(),
        ewmh_active_window,
        global_shortcuts_reliable: kind == SessionKind::X11 && x.is_some(),
        portal_global_shortcuts: portal_has("org.freedesktop.portal.GlobalShortcuts"),
        logind: bus(true).is_some_and(|c| has_owner(&c, "org.freedesktop.login1")),
    }
}

/// A bus connection whose method calls give up after 1 s, so a probe can
/// never hang the caller.
fn bus(system: bool) -> Option<zbus::blocking::Connection> {
    let b = if system {
        zbus::blocking::connection::Builder::system()
    } else {
        zbus::blocking::connection::Builder::session()
    };
    b.ok()?.method_timeout(Duration::from_secs(1)).build().ok()
}

fn has_owner(c: &zbus::blocking::Connection, name: &str) -> bool {
    zbus::blocking::fdo::DBusProxy::new(c)
        .ok()
        .and_then(|p| p.name_has_owner(name.try_into().ok()?).ok())
        .unwrap_or(false)
}

/// True if the running desktop portal exports `interface` (via introspection).
fn portal_has(interface: &str) -> bool {
    // Only a running portal is inspected: a probe must not D-Bus-activate it.
    let Some(conn) = bus(false).filter(|c| has_owner(c, "org.freedesktop.portal.Desktop")) else {
        return false;
    };
    let Ok(proxy) = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.DBus.Introspectable",
    ) else {
        return false;
    };
    proxy
        .call::<_, _, String>("Introspect", &())
        .is_ok_and(|xml| xml.contains(interface))
}

// ---------- X11 helpers ----------

fn atom(c: &impl Connection, name: &str) -> Option<Atom> {
    Some(
        c.intern_atom(false, name.as_bytes())
            .ok()?
            .reply()
            .ok()?
            .atom,
    )
}

fn prop32(c: &impl Connection, w: Window, name: &str, ty: impl Into<Atom>) -> Option<u32> {
    let a = atom(c, name)?;
    c.get_property(false, w, a, ty, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
}

fn active_window(c: &impl Connection, root: Window) -> Option<Window> {
    prop32(c, root, "_NET_ACTIVE_WINDOW", AtomEnum::WINDOW).filter(|w| *w != 0)
}

fn wm_class(c: &impl Connection, w: Window) -> Option<String> {
    let r = c
        .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    // "instance\0class\0": the class is the stable application name.
    let parts: Vec<&[u8]> = r
        .value
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .collect();
    parts
        .last()
        .map(|c| String::from_utf8_lossy(c).to_ascii_lowercase())
}

// ---------- target identity (window level only) ----------

#[derive(Debug)]
pub struct TargetSnapshot {
    pub app: Option<AppIdentity>,
    window: Option<u32>,
    /// Always Unknown: no editable-element identity on Linux yet.
    pub field: FieldClass,
}

fn current_window() -> Option<(u32, AppIdentity)> {
    let (c, s) = x11rb::connect(None).ok()?;
    let root = c.setup().roots[s].root;
    let w = active_window(&c, root)?;
    let pid = prop32(&c, w, "_NET_WM_PID", AtomEnum::CARDINAL);
    Some((
        w,
        AppIdentity {
            pid: pid.and_then(|p| i32::try_from(p).ok()).unwrap_or(-1),
            bundle_id: wm_class(&c, w),
            launch_time: pid.and_then(process_start_ticks),
        },
    ))
}

/// Process start time in clock ticks since boot (`/proc/<pid>/stat` field
/// 22): distinguishes a reused PID.
fn process_start_ticks(pid: u32) -> Option<f64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name may contain spaces; fields resume after the last ')'.
    let rest = &stat[stat.rfind(')')? + 2..];
    rest.split_whitespace().nth(19)?.parse::<f64>().ok()
}

pub fn capture_target() -> Result<TargetSnapshot, PlatformError> {
    let cur = current_window();
    Ok(TargetSnapshot {
        app: cur.as_ref().map(|(_, a)| a.clone()),
        window: cur.map(|(w, _)| w),
        field: FieldClass::Unknown,
    })
}

/// Reports app/window changes when observable, otherwise Unknown. Never
/// `Same`: window identity alone is not enough for automatic delivery.
pub fn check_target(start: &TargetSnapshot) -> TargetCheck {
    let (Some(app0), Some(w0)) = (&start.app, start.window) else {
        return TargetCheck::Unknown;
    };
    match current_window() {
        Some((_, app)) if app.pid != app0.pid || app.launch_time != app0.launch_time => {
            TargetCheck::AppChanged
        }
        Some((w, _)) if w != w0 => TargetCheck::WindowChanged,
        _ => TargetCheck::Unknown,
    }
}

pub fn physical_modifiers_down() -> Option<bool> {
    None
}

pub fn send_paste_chord() -> Result<DispatchOutcome, PlatformError> {
    Err(PlatformError::Unsupported)
}

pub fn focused_text_value() -> Option<String> {
    None
}

// ---------- permissions ----------

pub fn accessibility_permission() -> Permission {
    Permission::NotApplicable
}

/// Linux desktops have no per-app microphone grant outside sandboxes;
/// capture errors are reported by the audio layer.
pub fn permission_status(_kind: PermissionKind) -> PermissionStatus {
    PermissionStatus::NotApplicable
}

pub fn request_permission(kind: PermissionKind, done: Box<dyn FnOnce(PermissionStatus) + Send>) {
    done(permission_status(kind));
}

pub fn open_privacy_settings(_kind: PermissionKind) -> bool {
    false
}

// ---------- clipboard (X11 CLIPBOARD selection owner) ----------

static CLIPBOARD_GENERATION: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Takes ownership of `CLIPBOARD` and serves `text` (UTF-8) from a background
/// thread until another client takes ownership. Returns once ownership is
/// confirmed, within 500 ms.
pub fn clipboard_write_text(text: &str) -> Result<i64, PlatformError> {
    if text.len() > MAX_CLIPBOARD_BYTES || std::env::var_os("DISPLAY").is_none() {
        return Err(PlatformError::ClipboardFailed);
    }
    let data = text.as_bytes().to_vec();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("x11-clipboard".into())
        .spawn(move || serve_clipboard(data, &tx))
        .map_err(|_| PlatformError::ClipboardFailed)?;
    match rx.recv_timeout(CLIPBOARD_BUDGET) {
        Ok(true) => Ok(CLIPBOARD_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1),
        _ => Err(PlatformError::ClipboardFailed),
    }
}

fn serve_clipboard(data: Vec<u8>, ready: &mpsc::Sender<bool>) {
    let fail = || {
        let _ = ready.send(false);
    };
    let Ok((c, s)) = x11rb::connect(None) else {
        return fail();
    };
    let root = c.setup().roots[s].root;
    let (Some(clipboard), Some(targets), Some(utf8), Some(text_atom), Some(mime)) = (
        atom(&c, "CLIPBOARD"),
        atom(&c, "TARGETS"),
        atom(&c, "UTF8_STRING"),
        atom(&c, "TEXT"),
        atom(&c, "text/plain;charset=utf-8"),
    ) else {
        return fail();
    };
    let Ok(win) = c.generate_id() else {
        return fail();
    };
    if c.create_window(
        0,
        win,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new(),
    )
    .is_err()
    {
        return fail();
    }
    let _ = c.set_selection_owner(win, clipboard, x11rb::CURRENT_TIME);
    let owned = c
        .get_selection_owner(clipboard)
        .ok()
        .and_then(|r| r.reply().ok())
        .is_some_and(|r| r.owner == win);
    let _ = ready.send(owned);
    if !owned {
        return;
    }
    while let Ok(ev) = c.wait_for_event() {
        match ev {
            Event::SelectionRequest(req) => {
                let mut prop = if req.property == x11rb::NONE {
                    req.target
                } else {
                    req.property
                };
                let served = if req.target == targets {
                    c.change_property32(
                        PropMode::REPLACE,
                        req.requestor,
                        prop,
                        AtomEnum::ATOM,
                        &[targets, utf8, text_atom, mime],
                    )
                    .is_ok()
                } else if req.target == utf8 || req.target == text_atom || req.target == mime {
                    c.change_property8(PropMode::REPLACE, req.requestor, prop, utf8, &data)
                        .is_ok()
                } else {
                    false
                };
                if !served {
                    prop = x11rb::NONE;
                }
                let note = SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: req.time,
                    requestor: req.requestor,
                    selection: req.selection,
                    target: req.target,
                    property: prop,
                };
                let _ = c.send_event(false, req.requestor, EventMask::NO_EVENT, note);
                let _ = c.flush();
            }
            // Someone else copied: our text is no longer the clipboard.
            Event::SelectionClear(_) => break,
            _ => {}
        }
    }
    let _ = c.destroy_window(win);
    let _ = c.flush();
}

/// Reads the current `CLIPBOARD` as UTF-8 (used by tests and the probe).
pub fn clipboard_read_text() -> Option<String> {
    let (c, s) = x11rb::connect(None).ok()?;
    let root = c.setup().roots[s].root;
    let clipboard = atom(&c, "CLIPBOARD")?;
    let utf8 = atom(&c, "UTF8_STRING")?;
    let dest = atom(&c, "DICTATION_CLIP")?;
    let win = c.generate_id().ok()?;
    c.create_window(
        0,
        win,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new(),
    )
    .ok()?;
    c.convert_selection(win, clipboard, utf8, dest, x11rb::CURRENT_TIME)
        .ok()?;
    c.flush().ok()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match c.poll_for_event().ok()? {
            Some(Event::SelectionNotify(n)) if n.property == x11rb::NONE => return None,
            Some(Event::SelectionNotify(_)) => {
                let r = c
                    .get_property(
                        true,
                        win,
                        dest,
                        AtomEnum::ANY,
                        0,
                        u32::try_from(MAX_CLIPBOARD_BYTES / 4).unwrap_or(0),
                    )
                    .ok()?
                    .reply()
                    .ok()?;
                return String::from_utf8(r.value).ok();
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    None
}

// ---------- session state via systemd-logind ----------

pub struct SuspendWatch {
    _threads: Vec<std::thread::JoinHandle<()>>,
}

/// Sleep (`PrepareForSleep(true)`), shutdown (`PrepareForShutdown(true)`)
/// and screen lock (`Lock` on this session) call `on_suspend`. Without a
/// system bus or logind nothing is reported.
pub fn watch_suspend(on_suspend: std::sync::Arc<dyn Fn() + Send + Sync>) -> SuspendWatch {
    let mut threads = vec![];
    // Signal streams have no method timeout; only the session lookup does.
    let Some(conn) = bus(true) else {
        return SuspendWatch { _threads: threads };
    };
    for signal in ["PrepareForSleep", "PrepareForShutdown"] {
        let (conn, cb) = (conn.clone(), std::sync::Arc::clone(&on_suspend));
        if let Ok(t) = std::thread::Builder::new()
            .name(format!("logind-{signal}"))
            .spawn(move || {
                let Ok(p) = zbus::blocking::Proxy::new(
                    &conn,
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                ) else {
                    return;
                };
                let Ok(it) = p.receive_signal(signal) else {
                    return;
                };
                for msg in it {
                    if msg.body().deserialize::<bool>().unwrap_or(false) {
                        cb();
                    }
                }
            })
        {
            threads.push(t);
        }
    }
    // Lock is a signal on this session's object.
    let cb = std::sync::Arc::clone(&on_suspend);
    if let Ok(t) = std::thread::Builder::new()
        .name("logind-lock".into())
        .spawn(move || {
            let Ok(m) = zbus::blocking::Proxy::new(
                &conn,
                "org.freedesktop.login1",
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
            ) else {
                return;
            };
            let Ok(path) = m.call::<_, _, zbus::zvariant::OwnedObjectPath>(
                "GetSessionByPID",
                &(std::process::id(),),
            ) else {
                return;
            };
            let Ok(s) = zbus::blocking::Proxy::new(
                &conn,
                "org.freedesktop.login1",
                path,
                "org.freedesktop.login1.Session",
            ) else {
                return;
            };
            let Ok(it) = s.receive_signal("Lock") else {
                return;
            };
            for _ in it {
                cb();
            }
        })
    {
        threads.push(t);
    }
    SuspendWatch { _threads: threads }
}

/// Run under a real or virtual X server: `xvfb-run cargo test -p
/// dictation-platform -- --ignored`.
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn never_auto_delivers() {
        let s = capture_target().unwrap();
        assert_eq!(s.field, FieldClass::Unknown);
        assert!(!check_target(&s).allows_auto_delivery());
        assert!(send_paste_chord().is_err());
        assert!(!wait_modifiers_released(Duration::from_millis(1)));
    }

    #[test]
    fn probe_without_display_reports_none() {
        let p = probe();
        eprintln!("linux probe: {p:?}");
        if std::env::var_os("DISPLAY").is_none() {
            assert!(!p.x11_reachable);
            assert!(!p.global_shortcuts_reliable);
        }
    }

    #[test]
    fn suspend_watch_starts_without_blocking() {
        let t = std::time::Instant::now();
        let _w = watch_suspend(std::sync::Arc::new(|| {}));
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn proc_start_time_is_read() {
        assert!(process_start_ticks(std::process::id()).is_some_and(|t| t > 0.0));
    }

    #[test]
    #[ignore = "needs an X server (xvfb-run)"]
    fn x11_clipboard_roundtrip_and_ownership_handover() {
        let text = "Grüße — naïve café, 😀 漢字 ok";
        clipboard_write_text(text).unwrap();
        assert_eq!(clipboard_read_text().as_deref(), Some(text));
        // A newer copy replaces ours; the old server thread exits on SelectionClear.
        clipboard_write_text("second").unwrap();
        assert_eq!(clipboard_read_text().as_deref(), Some("second"));
        assert!(clipboard_write_text(&"x".repeat(MAX_CLIPBOARD_BYTES + 1)).is_err());
    }

    #[test]
    #[ignore = "needs an X server (xvfb-run)"]
    fn x11_window_identity_changes_are_reported() {
        // Simulate an EWMH window manager: publish _NET_ACTIVE_WINDOW on the
        // root and _NET_WM_PID / WM_CLASS on two windows.
        let (c, s) = x11rb::connect(None).unwrap();
        let root = c.setup().roots[s].root;
        let mk = |class: &[u8]| {
            let w = c.generate_id().unwrap();
            c.create_window(
                0,
                w,
                root,
                0,
                0,
                10,
                10,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap();
            c.change_property32(
                PropMode::REPLACE,
                w,
                atom(&c, "_NET_WM_PID").unwrap(),
                AtomEnum::CARDINAL,
                &[std::process::id()],
            )
            .unwrap();
            c.change_property8(
                PropMode::REPLACE,
                w,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                class,
            )
            .unwrap();
            w
        };
        let a = mk(b"editor\0Editor\0");
        let b = mk(b"other\0Other\0");
        let set_active = |w: u32| {
            c.change_property32(
                PropMode::REPLACE,
                root,
                atom(&c, "_NET_ACTIVE_WINDOW").unwrap(),
                AtomEnum::WINDOW,
                &[w],
            )
            .unwrap();
            c.flush().unwrap();
        };
        set_active(a);
        assert!(probe().ewmh_active_window);
        let t = capture_target().unwrap();
        assert_eq!(
            t.app.as_ref().and_then(|a| a.bundle_id.as_deref()),
            Some("editor")
        );
        assert_eq!(
            t.app.as_ref().map(|a| a.pid),
            i32::try_from(std::process::id()).ok()
        );
        assert_eq!(
            check_target(&t),
            TargetCheck::Unknown,
            "unchanged window is still not deliverable"
        );
        set_active(b);
        assert_eq!(check_target(&t), TargetCheck::WindowChanged);
    }
}
