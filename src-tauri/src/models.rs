//! Model commands (docs/04 §3: get_models, install_model, cancel_model_install,
//! import_model, remove_model; plus activate/unload/verify). The frontend can
//! only name a manifest id: URLs, paths and hashes come from the bundled
//! manifest and the app-private model directory. The import picker is opened
//! by Rust; webviews hold no dialog or filesystem permissions.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use dictation_coordinator::actor::{Command, ModelRef};
use dictation_models::download::HttpFetcher;
use dictation_models::{ApprovedModel, Cancel, ModelError, ModelState, ModelStatus, find};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;

const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    #[serde(flatten)]
    status: ModelStatus,
    active: bool,
    busy: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpFinished {
    id: &'static str,
    op: &'static str,
    error: Option<String>,
}

fn model(id: &str) -> Result<&'static ApprovedModel, String> {
    find(id).ok_or_else(|| "unknown model".to_string())
}

pub fn active_model(state: &AppState) -> Option<&'static ApprovedModel> {
    let current = state.model_ref.lock().ok()?.clone()?;
    dictation_models::APPROVED
        .iter()
        .find(|m| state.models.path(m) == current.path)
}

#[tauri::command]
pub fn get_models(state: State<'_, AppState>) -> Vec<ModelView> {
    let active = active_model(&state).map(|m| m.id);
    let ops = state
        .model_ops
        .lock()
        .map(|o| o.keys().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    state
        .models
        .inventory()
        .into_iter()
        .map(|status| ModelView {
            active: Some(status.model.id) == active,
            busy: ops.contains(&status.model.id),
            status,
        })
        .collect()
}

/// Registers an exclusive operation for `m`; returns its cancel flag.
fn begin_op(state: &AppState, m: &'static ApprovedModel) -> Result<Cancel, String> {
    let mut ops = state.model_ops.lock().map_err(|_| "state poisoned")?;
    if ops.contains_key(m.id) {
        return Err("BUSY".into());
    }
    let c = Cancel::new();
    ops.insert(m.id, c.clone());
    Ok(c)
}

fn end_op(
    app: &AppHandle,
    m: &'static ApprovedModel,
    op: &'static str,
    result: Result<(), ModelError>,
) {
    if let Some(state) = app.try_state::<AppState>()
        && let Ok(mut ops) = state.model_ops.lock()
    {
        ops.remove(m.id);
    }
    let _ = app.emit_to(
        "main",
        "model_op_finished",
        OpFinished {
            id: m.id,
            op,
            error: result.err().map(|e| e.to_string()),
        },
    );
}

fn progress_emitter(app: &AppHandle) -> impl FnMut(dictation_models::Progress) + use<> {
    let app = app.clone();
    let mut last: Option<Instant> = None;
    move |p| {
        if p.done == p.total || last.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL) {
            last = Some(Instant::now());
            let _ = app.emit_to("main", "model_progress", p);
        }
    }
}

/// After the first successful install/import, activate it if nothing is loaded.
fn activate_if_none(app: &AppHandle, m: &'static ApprovedModel) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if active_model(&state).is_none() {
        let _ = activate(&state, m);
        crate::refresh_readiness(&state);
    }
}

#[tauri::command]
pub fn install_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), String> {
    let m = model(&model_id)?;
    let cancel = begin_op(&state, m)?;
    std::thread::spawn(move || {
        let fetcher = HttpFetcher::new(concat!("LocalDictation/", env!("CARGO_PKG_VERSION")));
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let result = state
            .models
            .install(m, &fetcher, &mut progress_emitter(&app), &cancel);
        let ok = result.is_ok();
        end_op(&app, m, "install", result);
        if ok {
            activate_if_none(&app, m);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn import_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), String> {
    let m = model(&model_id)?;
    let cancel = begin_op(&state, m)?;
    let picker_app = app.clone();
    app.dialog()
        .file()
        .set_title(format!("Select {} ({})", m.display_name, m.file_name))
        .add_filter("Whisper model", &["bin"])
        .pick_file(move |picked| {
            let app = picker_app;
            let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
                return end_op(&app, m, "import", Err(ModelError::Canceled));
            };
            std::thread::spawn(move || {
                let Some(state) = app.try_state::<AppState>() else {
                    return;
                };
                let result = state
                    .models
                    .import(m, &path, &mut progress_emitter(&app), &cancel);
                let ok = result.is_ok();
                end_op(&app, m, "import", result);
                if ok {
                    activate_if_none(&app, m);
                }
            });
        });
    Ok(())
}

#[tauri::command]
pub fn verify_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), String> {
    let m = model(&model_id)?;
    let cancel = begin_op(&state, m)?;
    std::thread::spawn(move || {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let result = state.models.verify(m, &cancel);
        end_op(&app, m, "verify", result);
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_model_operation(state: State<'_, AppState>, model_id: String) -> Result<(), String> {
    let m = model(&model_id)?;
    if let Ok(ops) = state.model_ops.lock()
        && let Some(c) = ops.get(m.id)
    {
        c.set();
    }
    Ok(())
}

#[tauri::command]
pub fn remove_model(state: State<'_, AppState>, model_id: String) -> Result<(), String> {
    let m = model(&model_id)?;
    if active_model(&state).is_some_and(|a| a.id == m.id) {
        return Err("MODEL_ACTIVE".into());
    }
    if state.model_ops.lock().is_ok_and(|o| o.contains_key(m.id)) {
        return Err("BUSY".into());
    }
    state.models.remove(m).map_err(|e| e.to_string())
}

/// Loads and smoke-tests the model in a fresh worker; persists the choice
/// only on success. Blocking: call from a worker thread.
pub fn activate(state: &AppState, m: &'static ApprovedModel) -> Result<(), String> {
    if !matches!(state.models.state(m), ModelState::Installed { .. }) {
        return Err("MODEL_UNAVAILABLE".into());
    }
    let model = ModelRef {
        path: state.models.path(m),
        sha256: m.sha256.to_string(),
    };
    let (tx, rx) = mpsc::channel();
    state.handle.send(Command::SwitchModel { model, reply: tx });
    match rx.recv_timeout(Duration::from_secs(180)) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(format!("{e:?}")),
        Err(_) => return Err("timed out loading the model".into()),
    }
    if let Ok(mut store) = state.store.lock()
        && let Some(store) = store.as_mut()
    {
        let rev = store.get().revision;
        let _ = store.update(
            rev,
            &serde_json::json!({"transcription": {"model_id": m.id}}),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn activate_model(app: AppHandle, model_id: String) -> Result<(), String> {
    let m = model(&model_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let r = activate(&state, m);
        crate::refresh_readiness(&state);
        r
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn unload_model(state: State<'_, AppState>) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    state.handle.send(Command::UnloadModel { reply: tx });
    let r = rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "timed out".to_string())?;
    crate::refresh_readiness(&state);
    r.map_err(|e| format!("{e:?}"))
}
