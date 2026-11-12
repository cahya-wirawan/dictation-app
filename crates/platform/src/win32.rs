//! Windows adapter (docs/05 §3–§5): foreground HWND + process identity
//! (pid, creation time, image name) + UI Automation focused element (runtime
//! id, control type, password flag, read-only), Unicode clipboard and one
//! SendInput Ctrl+V.
//!
//! Safeguards specific to Windows:
//! * UIPI: SendInput into a higher-integrity process fails *silently*, so a
//!   target whose integrity level is above ours (or whose token cannot be
//!   read) is never eligible for automatic delivery (docs/05 §3, [S7]).
//! * A null foreground window (secure desktop, UAC prompt, lock screen) is
//!   `NoFrontmostApp`, never a target.
//! * Only synthetic keys sent by this transaction are released; if SendInput
//!   submits fewer events than requested the result is `Uncertain` and the
//!   control key we pressed is released.
//!
//! COM (UI Automation) is initialized per thread in the multithreaded
//! apartment; all target operations run on the delivery thread.

use windows::Win32::Foundation::GlobalFree;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HGLOBAL, HWND};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, SAFEARRAY,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetProcessTimes, OpenProcess, OpenProcessToken,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationValuePattern,
    UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_ValuePatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LWIN,
    VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetForegroundWindow, GetWindowThreadProcessId,
};
use windows::core::PWSTR;

use crate::*;

const CF_UNICODETEXT: u32 = 13;
const VK_V: VIRTUAL_KEY = VIRTUAL_KEY(0x56);
/// Clipboard locking is bounded (docs/05 §5 step 4).
const CLIPBOARD_BUDGET: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq)]
struct ElementId {
    pid: i32,
    runtime_id: Vec<i32>,
}

#[derive(Debug)]
pub struct TargetSnapshot {
    pub app: AppIdentity,
    /// Top-level window (as an integer; never leaves this crate's callers).
    hwnd: isize,
    pub field: FieldClass,
    element: Option<ElementId>,
    /// Integrity level is not above ours (UIPI permits input).
    input_allowed: bool,
}

/// Owns a process handle.
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: handle came from OpenProcess/OpenProcessToken and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Owns a SAFEARRAY returned by UI Automation.
struct SafeArray(*mut SAFEARRAY);
impl Drop for SafeArray {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: array returned to us by GetRuntimeId; destroyed once.
            let _ = unsafe { SafeArrayDestroy(self.0) };
        }
    }
}

// ---------- permissions ----------

/// Windows has no Accessibility-style grant for UIA or SendInput at equal
/// integrity, so automatic delivery is available without a prompt.
pub fn accessibility_permission() -> Permission {
    Permission::Granted
}

/// Microphone privacy (Settings → Privacy & security → Microphone) is read
/// from the per-user and machine consent store for desktop (non-packaged)
/// apps. A missing value means the OS default, which allows desktop apps.
pub fn permission_status(kind: PermissionKind) -> PermissionStatus {
    match kind {
        PermissionKind::Accessibility => PermissionStatus::Granted,
        PermissionKind::Microphone => microphone_consent(),
    }
}

fn read_consent(hive: windows::Win32::System::Registry::HKEY, sub: &str) -> Option<String> {
    use windows::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};
    let sub: Vec<u16> = sub.encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = "Value".encode_utf16().chain(Some(0)).collect();
    let mut buf = [0u16; 32];
    let mut len = u32::try_from(std::mem::size_of_val(&buf)).unwrap_or(0);
    // SAFETY: valid NUL-terminated strings and an output buffer of `len` bytes.
    let r = unsafe {
        RegGetValueW(
            hive,
            windows::core::PCWSTR(sub.as_ptr()),
            windows::core::PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if r.is_err() {
        return None;
    }
    let n = (len as usize / 2).min(buf.len());
    Some(
        String::from_utf16_lossy(&buf[..n])
            .trim_end_matches('\0')
            .to_owned(),
    )
}

fn microphone_consent() -> PermissionStatus {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    const BASE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    let nonpackaged = format!(r"{BASE}\NonPackaged");
    let values = [
        read_consent(HKEY_LOCAL_MACHINE, BASE),
        read_consent(HKEY_CURRENT_USER, BASE),
        read_consent(HKEY_CURRENT_USER, &nonpackaged),
    ];
    if values
        .iter()
        .flatten()
        .any(|v| v.eq_ignore_ascii_case("Deny"))
    {
        PermissionStatus::Denied
    } else {
        PermissionStatus::Granted
    }
}

/// There is no runtime prompt for desktop apps; report the current state.
pub fn request_permission(kind: PermissionKind, done: Box<dyn FnOnce(PermissionStatus) + Send>) {
    done(permission_status(kind));
}

pub fn open_privacy_settings(kind: PermissionKind) -> bool {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let uri = match kind {
        PermissionKind::Microphone => "ms-settings:privacy-microphone",
        PermissionKind::Accessibility => return false,
    };
    let op: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    let file: Vec<u16> = uri.encode_utf16().chain(Some(0)).collect();
    // SAFETY: NUL-terminated wide strings; a fixed settings URI, never user input.
    let r = unsafe {
        ShellExecuteW(
            None,
            windows::core::PCWSTR(op.as_ptr()),
            windows::core::PCWSTR(file.as_ptr()),
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    r.0 as isize > 32
}

// ---------- target identity ----------

fn filetime_f64(t: FILETIME) -> f64 {
    ((u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)) as f64
}

/// Integrity RID of a process token (e.g. 0x2000 medium, 0x3000 high).
fn integrity_rid(process: HANDLE) -> Option<u32> {
    let mut token = HANDLE::default();
    // SAFETY: valid process handle; token handle is owned below.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.ok()?;
    let token = Handle(token);
    let mut len = 0u32;
    // SAFETY: size query.
    let _ = unsafe { GetTokenInformation(token.0, TokenIntegrityLevel, None, 0, &mut len) };
    if len == 0 || len > 4096 {
        return None;
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: buffer of at least `len` bytes, 8-byte aligned.
    unsafe {
        GetTokenInformation(
            token.0,
            TokenIntegrityLevel,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut len,
        )
    }
    .ok()?;
    // SAFETY: the buffer now holds a TOKEN_MANDATORY_LABEL with a valid SID.
    unsafe {
        let label = &*(buf.as_ptr().cast::<TOKEN_MANDATORY_LABEL>());
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return None;
        }
        Some(*GetSidSubAuthority(sid, u32::from(count - 1)))
    }
}

fn own_integrity() -> Option<u32> {
    // SAFETY: pseudo-handle for the current process; not closed.
    integrity_rid(unsafe { GetCurrentProcess() })
}

struct ProcessFacts {
    launch_time: Option<f64>,
    image: Option<String>,
    integrity: Option<u32>,
}

fn process_facts(pid: u32) -> Option<ProcessFacts> {
    // SAFETY: limited query right only; handle owned below.
    let h = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?);
    let (mut c, mut e, mut k, mut u) = Default::default();
    // SAFETY: valid handle and out-pointers.
    let launch_time = unsafe { GetProcessTimes(h.0, &mut c, &mut e, &mut k, &mut u) }
        .ok()
        .map(|()| filetime_f64(c));
    let mut buf = [0u16; 1024];
    let mut len = u32::try_from(buf.len()).unwrap_or(0);
    // SAFETY: buffer of `len` UTF-16 units.
    let image = unsafe {
        QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
    }
    .ok()
    .and_then(|()| {
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        // App identity = lower-case executable file name (exclusion lists use this).
        full.rsplit(['\\', '/']).next().map(str::to_ascii_lowercase)
    });
    Some(ProcessFacts {
        launch_time,
        image,
        integrity: integrity_rid(h.0),
    })
}

fn automation() -> Option<IUIAutomation> {
    // SAFETY: idempotent per thread; S_FALSE / RPC_E_CHANGED_MODE are fine.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    // SAFETY: standard in-proc COM activation of the UIA client.
    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()
}

fn runtime_id(el: &IUIAutomationElement) -> Option<Vec<i32>> {
    // SAFETY: returns a new SAFEARRAY of VT_I4 that we own.
    let sa = SafeArray(unsafe { el.GetRuntimeId() }.ok()?);
    if sa.0.is_null() {
        return None;
    }
    // SAFETY: one-dimensional array from UIA.
    let (lo, hi) = unsafe {
        (
            SafeArrayGetLBound(sa.0, 1).ok()?,
            SafeArrayGetUBound(sa.0, 1).ok()?,
        )
    };
    if hi < lo || hi - lo > 64 {
        return None;
    }
    let mut v = Vec::with_capacity((hi - lo + 1) as usize);
    for i in lo..=hi {
        let mut x = 0i32;
        // SAFETY: index within bounds; element type is i32.
        unsafe { SafeArrayGetElement(sa.0, &i, (&mut x as *mut i32).cast()) }.ok()?;
        v.push(x);
    }
    Some(v)
}

fn classify(el: &IUIAutomationElement) -> FieldClass {
    // SAFETY: COM property getters on a live element.
    unsafe {
        if el.CurrentIsPassword().is_ok_and(|b| b.as_bool()) {
            return FieldClass::Secure;
        }
        if el.CurrentIsEnabled().is_ok_and(|b| !b.as_bool()) {
            return FieldClass::NotEditable;
        }
        let Ok(ct) = el.CurrentControlType() else {
            return FieldClass::Unknown;
        };
        if ct != UIA_EditControlTypeId && ct != UIA_DocumentControlTypeId {
            return FieldClass::NotEditable;
        }
        if let Ok(vp) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            && vp.CurrentIsReadOnly().is_ok_and(|b| b.as_bool())
        {
            return FieldClass::NotEditable;
        }
        FieldClass::Editable
    }
}

fn focused_element() -> Option<(IUIAutomationElement, ElementId)> {
    let uia = automation()?;
    // SAFETY: COM call on a live automation object.
    let el = unsafe { uia.GetFocusedElement() }.ok()?;
    // SAFETY: property getter.
    let pid = unsafe { el.CurrentProcessId() }.ok()?;
    let runtime_id = runtime_id(&el)?;
    Some((el, ElementId { pid, runtime_id }))
}

pub fn capture_target() -> Result<TargetSnapshot, PlatformError> {
    // SAFETY: plain query.
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() {
        return Err(PlatformError::NoFrontmostApp); // secure desktop, UAC, lock screen
    }
    // SAFETY: valid HWND.
    let root = unsafe { GetAncestor(fg, GA_ROOT) };
    let hwnd = if root.0.is_null() { fg } else { root };
    let mut pid = 0u32;
    // SAFETY: valid HWND and out-pointer.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return Err(PlatformError::NoFrontmostApp);
    }
    let facts = process_facts(pid);
    let ours = own_integrity();
    let input_allowed =
        matches!((facts.as_ref().and_then(|f| f.integrity), ours), (Some(t), Some(o)) if t <= o);
    let (field, element) = match focused_element() {
        Some((el, id)) if u32::try_from(id.pid).ok() == Some(pid) => (classify(&el), Some(id)),
        // Focus belongs to another process than the foreground window, or UIA
        // gave no identity: unknown is not evidence of a safe field.
        _ => (FieldClass::Unknown, None),
    };
    Ok(TargetSnapshot {
        app: AppIdentity {
            pid: i32::try_from(pid).unwrap_or(-1),
            bundle_id: facts.as_ref().and_then(|f| f.image.clone()),
            launch_time: facts.as_ref().and_then(|f| f.launch_time),
        },
        hwnd: hwnd.0 as isize,
        field: if input_allowed {
            field
        } else {
            FieldClass::Unknown
        },
        element,
        input_allowed,
    })
}

pub fn check_target(start: &TargetSnapshot) -> TargetCheck {
    // SAFETY: plain query.
    if u32::try_from(start.app.pid).ok() == Some(unsafe { GetCurrentProcessId() }) {
        return TargetCheck::OwnApp;
    }
    if !start.input_allowed {
        return TargetCheck::Unknown; // higher integrity or unreadable token (UIPI)
    }
    let Ok(now) = capture_target() else {
        return TargetCheck::Unknown;
    };
    if now.app.pid != start.app.pid || now.app.launch_time != start.app.launch_time {
        return TargetCheck::AppChanged;
    }
    if now.hwnd != start.hwnd {
        return TargetCheck::WindowChanged;
    }
    match (&start.element, &now.element) {
        (Some(a), Some(b)) if a != b => return TargetCheck::FieldChanged,
        (Some(_), Some(_)) => {}
        _ => return TargetCheck::Unknown,
    }
    match now.field {
        FieldClass::Editable => TargetCheck::Same,
        FieldClass::Secure => TargetCheck::Secure,
        FieldClass::NotEditable => TargetCheck::NotEditable,
        FieldClass::Unknown => TargetCheck::Unknown,
    }
}

// ---------- input ----------

pub fn physical_modifiers_down() -> Option<bool> {
    let down = |vk: VIRTUAL_KEY| {
        // SAFETY: plain key-state query; the high bit means currently down.
        (unsafe { GetAsyncKeyState(i32::from(vk.0)) } as u16 & 0x8000) != 0
    };
    Some(
        [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
            .into_iter()
            .any(down),
    )
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    // SAFETY: pure table lookup.
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) } as u16;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// One Ctrl+V. `Sent` means SendInput accepted all four events; it does not
/// prove the destination received text.
pub fn send_paste_chord() -> Result<DispatchOutcome, PlatformError> {
    let seq = [
        key(VK_CONTROL, false),
        key(VK_V, false),
        key(VK_V, true),
        key(VK_CONTROL, true),
    ];
    let size = i32::try_from(std::mem::size_of::<INPUT>()).unwrap_or(0);
    // SAFETY: valid INPUT array.
    let n = unsafe { SendInput(&seq, size) };
    if n as usize == seq.len() {
        return Ok(DispatchOutcome::Sent);
    }
    // Partial submission: release only the synthetic keys this call pressed.
    if n >= 1 {
        let release: &[INPUT] = if n >= 2 {
            &[key(VK_V, true), key(VK_CONTROL, true)]
        } else {
            &[key(VK_CONTROL, true)]
        };
        // SAFETY: as above.
        let _ = unsafe { SendInput(release, size) };
    }
    Ok(DispatchOutcome::Uncertain)
}

// ---------- clipboard ----------

/// Writes plain Unicode text. Retries opening the clipboard for at most
/// 500 ms (another app may hold it). Returns the clipboard sequence number.
pub fn clipboard_write_text(text: &str) -> Result<i64, PlatformError> {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let deadline = std::time::Instant::now() + CLIPBOARD_BUDGET;
    loop {
        // SAFETY: no owner window; closed below on every path.
        if unsafe { OpenClipboard(None) }.is_ok() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(PlatformError::ClipboardFailed);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let result = (|| {
        // SAFETY: clipboard is open on this thread.
        unsafe { EmptyClipboard() }.map_err(|_| PlatformError::ClipboardFailed)?;
        let bytes = wide.len() * 2;
        // SAFETY: movable global allocation that the clipboard takes ownership of on success.
        let mem: HGLOBAL = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }
            .map_err(|_| PlatformError::ClipboardFailed)?;
        // SAFETY: lock the allocation we just made.
        let ptr = unsafe { GlobalLock(mem) };
        if ptr.is_null() {
            // SAFETY: not handed to the clipboard.
            let _ = unsafe { GlobalFree(Some(mem)) };
            return Err(PlatformError::ClipboardFailed);
        }
        // SAFETY: `ptr` points to `bytes` writable bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr.cast::<u16>(), wide.len());
            let _ = GlobalUnlock(mem);
        }
        // SAFETY: on success the system owns `mem`; on failure we free it.
        if unsafe { SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0))) }.is_err() {
            let _ = unsafe { GlobalFree(Some(mem)) };
            return Err(PlatformError::ClipboardFailed);
        }
        Ok(())
    })();
    // SAFETY: we opened it above.
    let _ = unsafe { CloseClipboard() };
    result?;
    // SAFETY: plain query.
    Ok(i64::from(unsafe { GetClipboardSequenceNumber() }))
}

/// Development probe only: the focused element's UIA value.
pub fn focused_text_value() -> Option<String> {
    let (el, _) = focused_element()?;
    // SAFETY: COM calls on a live element.
    unsafe {
        let vp = el
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()?;
        vp.CurrentValue().ok().map(|b| b.to_string())
    }
}

// ---------- session state (docs/04 invariant 10) ----------

thread_local! {
    static ON_SUSPEND: std::cell::RefCell<Option<std::sync::Arc<dyn Fn() + Send + Sync>>> =
        const { std::cell::RefCell::new(None) };
}

extern "system" fn session_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, PBT_APMSUSPEND, PostQuitMessage, WM_DESTROY, WM_ENDSESSION,
        WM_POWERBROADCAST, WM_QUERYENDSESSION, WM_WTSSESSION_CHANGE, WTS_CONSOLE_DISCONNECT,
        WTS_REMOTE_DISCONNECT, WTS_SESSION_LOCK, WTS_SESSION_LOGOFF,
    };
    let fire = match msg {
        WM_WTSSESSION_CHANGE => matches!(
            wparam.0 as u32,
            WTS_SESSION_LOCK | WTS_SESSION_LOGOFF | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT
        ),
        WM_POWERBROADCAST => wparam.0 as u32 == PBT_APMSUSPEND,
        WM_QUERYENDSESSION | WM_ENDSESSION => true,
        _ => false,
    };
    if fire {
        ON_SUSPEND.with(|c| {
            if let Some(cb) = c.borrow().as_ref() {
                cb();
            }
        });
    }
    if msg == WM_DESTROY {
        // SAFETY: ends this thread's message loop.
        unsafe { PostQuitMessage(0) };
    }
    if msg == WM_QUERYENDSESSION {
        return windows::Win32::Foundation::LRESULT(1); // never block logoff/shutdown
    }
    // SAFETY: default handling for everything else.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A hidden top-level window on its own thread that receives lock, logoff,
/// remote/console disconnect, suspend and end-session messages. Dropping it
/// closes the window and joins the thread.
pub struct SuspendWatch {
    hwnd: std::sync::Arc<std::sync::atomic::AtomicIsize>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub fn watch_suspend(on_suspend: std::sync::Arc<dyn Fn() + Send + Sync>) -> SuspendWatch {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::RemoteDesktop::{
        NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, MSG, RegisterClassW, TranslateMessage,
        WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPED,
    };
    let slot = std::sync::Arc::new(std::sync::atomic::AtomicIsize::new(0));
    let out = std::sync::Arc::clone(&slot);
    let thread = std::thread::Builder::new()
        .name("session-watch".into())
        .spawn(move || {
            ON_SUSPEND.with(|c| *c.borrow_mut() = Some(on_suspend));
            let class: Vec<u16> = "LocalDictationSessionWatch"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // SAFETY: module handle of this executable.
            let Ok(module) = (unsafe { GetModuleHandleW(None) }) else {
                return;
            };
            let wc = WNDCLASSW {
                lpfnWndProc: Some(session_wndproc),
                hInstance: module.into(),
                lpszClassName: windows::core::PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            // SAFETY: valid class description; re-registration failure is harmless.
            unsafe { RegisterClassW(&wc) };
            // SAFETY: hidden (never shown) top-level window: broadcast messages
            // such as WM_POWERBROADCAST are not delivered to message-only windows.
            let Ok(hwnd) = (unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    windows::core::PCWSTR(class.as_ptr()),
                    windows::core::PCWSTR(class.as_ptr()),
                    WS_OVERLAPPED,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(module.into()),
                    None,
                )
            }) else {
                return;
            };
            out.store(hwnd.0 as isize, std::sync::atomic::Ordering::SeqCst);
            // SAFETY: valid window owned by this thread.
            let _ = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) };
            let mut msg = MSG::default();
            // SAFETY: standard message loop for this thread's window.
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        })
        .ok();
    SuspendWatch { hwnd: slot, thread }
}

impl Drop for SuspendWatch {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        let h = self.hwnd.load(std::sync::atomic::Ordering::SeqCst);
        if h != 0 {
            // SAFETY: window created by the watch thread; WM_CLOSE → WM_DESTROY → quit.
            let _ = unsafe {
                PostMessageW(
                    Some(HWND(h as *mut _)),
                    WM_CLOSE,
                    Default::default(),
                    Default::default(),
                )
            };
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}

/// Native end-to-end checks on a real interactive desktop (CI: windows
/// runner). Run with `cargo test -p dictation-platform -- --ignored`.
/// They use this test process's own Edit controls as the receiving app, so
/// the real clipboard, SendInput and UI Automation paths are exercised and
/// the received text is read back from the control itself.
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, ES_MULTILINE, ES_PASSWORD,
        GetWindowTextLengthW, GetWindowTextW, MSG, PM_REMOVE, PeekMessageW, SW_SHOW,
        SetForegroundWindow, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD,
        WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };
    use windows::core::w;

    fn pump(ms: u64) {
        let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        let mut msg = MSG::default();
        while std::time::Instant::now() < end {
            // SAFETY: message pump for windows owned by this thread.
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn text_of(h: HWND) -> String {
        // SAFETY: valid window; buffer sized from the reported length.
        unsafe {
            let n = GetWindowTextLengthW(h);
            let mut buf = vec![0u16; usize::try_from(n).unwrap() + 1];
            let got = GetWindowTextW(h, &mut buf);
            String::from_utf16_lossy(&buf[..usize::try_from(got).unwrap()])
        }
    }

    struct Rig {
        top: HWND,
        edit: HWND,
        other: HWND,
        password: HWND,
    }

    fn rig() -> Rig {
        // SAFETY: plain window creation on this thread, destroyed at the end.
        unsafe {
            let top = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("dictation e2e"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100,
                100,
                500,
                300,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let edit_style =
                |extra: i32| WS_CHILD | WS_VISIBLE | WINDOW_STYLE(u32::try_from(extra).unwrap());
            let mk = |y: i32, extra: i32| {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("EDIT"),
                    w!(""),
                    edit_style(extra),
                    10,
                    y,
                    400,
                    40,
                    Some(top),
                    None,
                    None,
                    None,
                )
                .unwrap()
            };
            let edit = mk(10, ES_MULTILINE);
            let other = mk(60, ES_MULTILINE);
            let password = mk(110, ES_PASSWORD);
            let _ = ShowWindow(top, SW_SHOW);
            // A process may only take the foreground right after input; a
            // synthetic Alt tap satisfies the foreground-lock rule.
            let alt = [key(VK_MENU, false), key(VK_MENU, true)];
            let _ = SendInput(&alt, i32::try_from(std::mem::size_of::<INPUT>()).unwrap());
            let _ = SetForegroundWindow(top);
            pump(300);
            Rig {
                top,
                edit,
                other,
                password,
            }
        }
    }

    fn focus(h: HWND) {
        // SAFETY: child of our foreground window.
        let _ = unsafe { SetFocus(Some(h)) };
        pump(200);
    }

    #[test]
    #[ignore = "needs an interactive Windows desktop"]
    fn real_clipboard_sendinput_and_uia_roundtrip() {
        let r = rig();
        // SAFETY: plain query.
        assert_eq!(
            unsafe { GetForegroundWindow() },
            r.top,
            "test window did not become foreground"
        );

        focus(r.edit);
        let t = capture_target().unwrap();
        eprintln!("captured: {t:?}");
        assert_eq!(t.field, FieldClass::Editable);
        assert!(t.input_allowed, "same-integrity target must allow input");
        assert_eq!(
            t.app.bundle_id.as_deref().map(|s| s.ends_with(".exe")),
            Some(true)
        );
        // This process is the app here, and the app itself is never a target.
        assert_eq!(check_target(&t), TargetCheck::OwnApp);

        // Field identity changes when focus moves inside the same window.
        focus(r.other);
        let t2 = capture_target().unwrap();
        assert_eq!(t2.hwnd, t.hwnd);
        assert_ne!(
            t2.element, t.element,
            "UIA runtime id must distinguish fields"
        );
        focus(r.password);
        assert_eq!(capture_target().unwrap().field, FieldClass::Secure);

        // Unicode through the real clipboard and one real Ctrl+V.
        focus(r.edit);
        let text = "Grüße — naïve café, 😀 漢字 ok";
        assert!(physical_modifiers_down() == Some(false));
        let seq = clipboard_write_text(text).unwrap();
        assert!(seq > 0);
        assert_eq!(send_paste_chord().unwrap(), DispatchOutcome::Sent);
        pump(500);
        let got = text_of(r.edit);
        assert_eq!(got, text, "destination did not receive the exact text once");
        assert_eq!(
            focused_text_value().as_deref(),
            Some(text),
            "UIA value readback"
        );
        assert!(text_of(r.other).is_empty(), "paste reached the wrong field");
        eprintln!("RECEIVED=true DUPLICATED=false text={got:?}");

        // SAFETY: windows created above.
        unsafe {
            let _ = DestroyWindow(r.top);
        }
    }

    #[test]
    fn own_integrity_is_readable() {
        let rid = own_integrity().expect("token integrity");
        assert!(rid >= 0x1000, "integrity rid {rid:#x}");
    }

    #[test]
    fn suspend_watch_starts_and_stops() {
        let w = watch_suspend(std::sync::Arc::new(|| {}));
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_ne!(
            w.hwnd.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "hidden session window created"
        );
        drop(w); // joins the thread
    }
}
