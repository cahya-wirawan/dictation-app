//! M4 commands: permissions, shortcut capture with a real registration check,
//! onboarding completion, launch at login and the review window.

use std::sync::mpsc;
use std::time::Duration;

use dictation_platform::{PermissionKind, PermissionStatus};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::AppState;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Granted,
    Denied,
    NotDetermined,
    Restricted,
    NotApplicable,
}

impl From<PermissionStatus> for Status {
    fn from(s: PermissionStatus) -> Self {
        match s {
            PermissionStatus::Granted => Status::Granted,
            PermissionStatus::Denied => Status::Denied,
            PermissionStatus::NotDetermined => Status::NotDetermined,
            PermissionStatus::Restricted => Status::Restricted,
            PermissionStatus::NotApplicable => Status::NotApplicable,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    microphone: Status,
    accessibility: Status,
}

fn kind(k: &str) -> Result<PermissionKind, String> {
    match k {
        "microphone" => Ok(PermissionKind::Microphone),
        "accessibility" => Ok(PermissionKind::Accessibility),
        _ => Err("unknown permission".into()),
    }
}

/// Reads OS state every time; grants are never cached or persisted.
#[tauri::command]
pub fn check_permissions() -> Permissions {
    Permissions {
        microphone: dictation_platform::permission_status(PermissionKind::Microphone).into(),
        accessibility: dictation_platform::permission_status(PermissionKind::Accessibility).into(),
    }
}

/// User-initiated only (an explicit button in onboarding/settings).
#[tauri::command]
pub async fn request_permission(permission: String) -> Result<Status, String> {
    let k = kind(&permission)?;
    tauri::async_runtime::spawn_blocking(move || {
        let (tx, rx) = mpsc::channel();
        dictation_platform::request_permission(
            k,
            Box::new(move |s| {
                let _ = tx.send(s);
            }),
        );
        // The microphone prompt waits for the user; give up waiting, not the request.
        rx.recv_timeout(Duration::from_secs(120))
            .map(Status::from)
            .unwrap_or_else(|_| dictation_platform::permission_status(k).into())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_privacy_settings(permission: String) -> Result<bool, String> {
    Ok(dictation_platform::open_privacy_settings(kind(
        &permission,
    )?))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutResult {
    chord: Option<String>,
}

/// Registers `chord` for `slot` with the OS first and persists it only if the
/// registration succeeded (docs/02 §2 step 4). On failure the previous chord
/// is restored. `chord: None` clears the optional paste-draft chord.
#[tauri::command]
pub fn set_shortcut(
    app: AppHandle,
    state: State<'_, AppState>,
    slot: String,
    chord: Option<String>,
) -> Result<ShortcutResult, String> {
    let (field, cell) = match slot.as_str() {
        "toggle" => ("toggle", &state.toggle),
        "cancel" => ("cancel", &state.cancel),
        "pasteDraft" => ("paste_draft", &state.paste_draft),
        _ => return Err("unknown shortcut slot".into()),
    };
    if chord.is_none() && field != "paste_draft" {
        return Err("this shortcut is required".into());
    }
    let parsed = match &chord {
        Some(c) => Some(
            c.parse::<Shortcut>()
                .map_err(|e| format!("invalid shortcut: {e}"))?,
        ),
        None => None,
    };
    // Distinct from the other registered chords.
    for other in [&state.toggle, &state.cancel, &state.paste_draft] {
        if std::ptr::eq(other, cell) {
            continue;
        }
        if let (Some(p), Ok(o)) = (parsed, other.lock())
            && o.is_some_and(|o| o == p)
        {
            return Err("SHORTCUT_CONFLICT: already used by another dictation shortcut".into());
        }
    }
    let gs = app.global_shortcut();
    let previous = cell.lock().ok().and_then(|c| *c);
    if let Some(prev) = previous {
        let _ = gs.unregister(prev);
    }
    if let Some(p) = parsed
        && let Err(e) = gs.register(p)
    {
        if let Some(prev) = previous {
            let _ = gs.register(prev);
        }
        return Err(format!("SHORTCUT_CONFLICT: {e}"));
    }
    if let Ok(mut c) = cell.lock() {
        *c = parsed;
    }
    // Persist with the user's spelling (normalized by Rust's validation).
    let mut store = state.store.lock().map_err(|_| "state poisoned")?;
    if let Some(store) = store.as_mut() {
        let rev = store.get().revision;
        store
            .update(rev, &serde_json::json!({ "shortcuts": { field: chord } }))
            .map_err(|e| e.to_string())?;
    }
    if let Ok(mut err) = state.shortcut_error.lock() {
        *err = None;
    }
    Ok(ShortcutResult { chord })
}

/// Temporarily releases the global chords so pressing them in the capture
/// field does not start dictation; `resume_shortcuts` restores them.
#[tauri::command]
pub fn pause_shortcuts(app: AppHandle, state: State<'_, AppState>) {
    let gs = app.global_shortcut();
    for cell in [&state.toggle, &state.cancel, &state.paste_draft] {
        if let Ok(c) = cell.lock()
            && let Some(s) = *c
        {
            let _ = gs.unregister(s);
        }
    }
}

#[tauri::command]
pub fn resume_shortcuts(app: AppHandle, state: State<'_, AppState>) {
    let gs = app.global_shortcut();
    for cell in [&state.toggle, &state.cancel, &state.paste_draft] {
        if let Ok(c) = cell.lock()
            && let Some(s) = *c
        {
            let _ = gs.register(s);
        }
    }
}

#[tauri::command]
pub fn complete_onboarding(state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|_| "state poisoned")?;
    let store = store.as_mut().ok_or("settings unavailable")?;
    let rev = store.get().revision;
    store
        .update(
            rev,
            &serde_json::json!({"ui": {"onboarding_complete": true}}),
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Opt-in only (docs/01 §5). Uses a per-user LaunchAgent on macOS; no Apple
/// Events automation permission is involved.
#[tauri::command]
pub fn set_launch_at_login(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<bool, String> {
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())?;
    let actual = al.is_enabled().unwrap_or(enabled);
    let mut store = state.store.lock().map_err(|_| "state poisoned")?;
    if let Some(store) = store.as_mut() {
        let rev = store.get().revision;
        let _ = store.update(rev, &serde_json::json!({"ui": {"launch_at_login": actual}}));
    }
    Ok(actual)
}

/// Opens the review window. It deliberately takes focus, and opening it
/// revokes automatic delivery for an in-flight session (docs/02 §1).
#[tauri::command]
pub fn open_review(app: AppHandle) {
    crate::show_review(&app);
}

pub fn first_run_show(app: &AppHandle) {
    let needs = app
        .try_state::<AppState>()
        .and_then(|s| {
            s.store
                .lock()
                .ok()
                .and_then(|st| st.as_ref().map(|st| !st.get().ui.onboarding_complete))
        })
        .unwrap_or(true);
    if needs {
        crate::show_settings(app);
    }
}
