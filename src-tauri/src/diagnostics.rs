//! Support diagnostics (docs/10 §5): a redacted report the user previews and
//! saves explicitly. Nothing is uploaded. The save dialog is opened by Rust;
//! the webview never receives a path or file permission.

use dictation_coordinator::diagnostics::{Report, ReportFacts, SettingsSummary};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;

fn build(app: &AppHandle, state: &AppState) -> Report {
    let cfg = state
        .store
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(|s| s.get().clone()))
        .unwrap_or_default();
    let perms = crate::onboarding::check_permissions();
    let readiness = crate::readiness(state)
        .err()
        .and_then(|e| serde_json::to_value(e).ok());
    let installed = dictation_models::APPROVED
        .iter()
        .filter(|m| state.models.installed_path(m).is_some())
        .map(|m| m.id)
        .collect();
    state.diagnostics.report(ReportFacts {
        app_version: app.package_info().version.to_string(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        protocol_version: transcription_protocol::PROTOCOL_VERSION,
        active_model: crate::models::active_model(state).map(|m| m.id),
        installed_models: installed,
        permissions: serde_json::to_value(perms).unwrap_or_default(),
        readiness,
        settings: SettingsSummary {
            schema_version: cfg.schema_version,
            delivery_mode: serde_json::to_value(cfg.delivery.mode).unwrap_or_default(),
            language: cfg.transcription.language.clone(),
            silence_ms: cfg.audio.silence_ms,
            max_capture_ms: cfg.audio.max_capture_ms,
            custom_input_device: cfg.audio.device_id.is_some(),
            excluded_app_count: cfg.delivery.excluded_apps.len(),
            show_hud: cfg.ui.show_hud,
        },
    })
}

fn render(app: &AppHandle, state: &AppState) -> Result<String, String> {
    serde_json::to_string_pretty(&build(app, state)).map_err(|e| e.to_string())
}

/// The exact text that Save would write, for the preview.
#[tauri::command]
pub fn get_diagnostics(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    render(&app, &state)
}

/// Opens a native save dialog and writes a freshly generated report.
/// Resolves to `true` when saved, `false` when the user canceled.
#[tauri::command]
pub async fn save_diagnostics(app: AppHandle) -> Result<bool, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_title("Save diagnostics")
        .set_file_name("local-dictation-diagnostics.json")
        .add_filter("JSON", &["json"])
        .save_file(move |p| {
            let _ = tx.send(p.and_then(|p| p.into_path().ok()));
        });
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| e.to_string())?;
    let Some(path) = picked else {
        return Ok(false);
    };
    let state = app.state::<AppState>();
    let text = render(&app, &state)?;
    std::fs::write(&path, text).map_err(|e| format!("could not save: {e}"))?;
    Ok(true)
}
