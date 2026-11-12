//! Tauri shell (M1): tray, global shortcuts, HUD, settings window and the IPC
//! surface over the Rust coordinator.
//!
//! The coordinator (crates/coordinator) owns every session decision. This
//! file only translates OS/UI events into coordinator commands and coordinator
//! notifications into scoped window events. There is deliberately no command
//! that pastes, types, runs a process, reads an arbitrary path or downloads a
//! URL. Automatic delivery is not enabled yet (M3): sessions end in a
//! memory-only draft that the user can copy.

mod mictest;
mod models;
mod selftest;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dictation_audio::detector::DetectorConfig;
use dictation_audio::endpoint::EndpointConfig;
use dictation_coordinator::actor::ModelRef;
use dictation_coordinator::actor::{Adapters, Command, Coordinator, CoordinatorHandle, Notifier};
use dictation_coordinator::adapters::{
    CaptureSettings, CopyOnlyDelivery, DspCapture, SharedCaptureSettings, WorkerConfig,
    WorkerTranscriber, live_source,
};
use dictation_coordinator::config_store::{ConfigStore, LoadOutcome};
use dictation_domain::chord::ChordFilter;
use dictation_domain::config::Config;
use dictation_domain::session::{AppError, DraftReason, Outcome};
use dictation_domain::{Phase, SessionId};
use dictation_models::{Cancel, ModelStore};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use transcription_protocol::Language;

use mictest::MicTest;

const IPC_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) struct AppState {
    coordinator: Mutex<Option<Coordinator>>,
    pub(crate) handle: CoordinatorHandle,
    pub(crate) store: Mutex<Option<ConfigStore>>,
    pub(crate) models: ModelStore,
    /// Shared with the worker adapter: the active model (None = unloaded).
    pub(crate) model_ref: Arc<Mutex<Option<ModelRef>>>,
    pub(crate) model_ops: Mutex<HashMap<&'static str, Cancel>>,
    config_note: Option<String>,
    capture_settings: SharedCaptureSettings,
    language: Arc<Mutex<Language>>,
    toggle: Mutex<Option<Shortcut>>,
    cancel: Mutex<Option<Shortcut>>,
    toggle_filter: Mutex<ChordFilter>,
    cancel_filter: Mutex<ChordFilter>,
    shortcut_error: Mutex<Option<String>>,
    mic_test: Mutex<Option<MicTest>>,
    last_probe: Mutex<Option<ProbeReport>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeReport {
    before: String,
    after_hud_check: String,
    auto_delivery_would_be_allowed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DraftMeta {
    id: String,
    revision: u32,
    attempted: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    schema_version: u32,
    app_version: String,
    platform: &'static str,
    /// `None` if the coordinator did not answer in time.
    phase: Option<Phase>,
    draft: Option<DraftMeta>,
    ready_error: Option<AppError>,
    accessibility: String,
    worker_found: bool,
    active_model: Option<&'static str>,
    settings_revision: u64,
    settings_note: Option<String>,
    shortcuts: ShortcutStatus,
    mic_test_active: bool,
    last_probe: Option<ProbeReport>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShortcutStatus {
    toggle: String,
    cancel: String,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DraftView {
    id: String,
    revision: u32,
    text: String,
    reason: DraftReason,
    attempted: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PhasePayload {
    phase: Phase,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OutcomePayload {
    outcome: Outcome,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LevelPayload {
    level: f32,
}

/// Scoped fan-out: phase and level go to the HUD, phase and outcomes to the
/// settings window. No payload contains transcript text.
struct TauriNotifier {
    app: AppHandle,
}

impl Notifier for TauriNotifier {
    fn phase(&mut self, phase: Phase) {
        let _ = self
            .app
            .emit_to("hud", "phase_changed", PhasePayload { phase });
        let _ = self
            .app
            .emit_to("main", "phase_changed", PhasePayload { phase });
        let busy = matches!(
            phase,
            Phase::Starting
                | Phase::Listening
                | Phase::Finalizing
                | Phase::Transcribing
                | Phase::Delivering
        );
        let show_hud = self
            .app
            .try_state::<AppState>()
            .and_then(|s| {
                s.store
                    .lock()
                    .ok()
                    .and_then(|st| st.as_ref().map(|st| st.get().ui.show_hud))
            })
            .unwrap_or(true);
        if let Some(hud) = self.app.get_webview_window("hud") {
            let _ = if busy && show_hud {
                hud.show()
            } else {
                hud.hide()
            };
        }
    }

    fn outcome(&mut self, outcome: Outcome) {
        let _ = self
            .app
            .emit_to("main", "outcome", OutcomePayload { outcome });
    }

    fn level(&mut self, level: f32) {
        let _ = self.app.emit_to("hud", "mic_level", LevelPayload { level });
    }
}

/// Readiness for new sessions: worker binary present and an active model
/// that is still installed with the approved size.
fn readiness(state: &AppState) -> Result<(), AppError> {
    if selftest::worker_path().is_none() {
        return Err(AppError::NotReady);
    }
    match models::active_model(state) {
        Some(m) if state.models.installed_path(m).is_some() => Ok(()),
        Some(_) => Err(AppError::ModelInvalid),
        None => Err(AppError::ModelUnavailable),
    }
}

pub(crate) fn refresh_readiness(state: &AppState) {
    state.handle.send(Command::SetReady(readiness(state)));
}

fn endpoint_from(cfg: &Config) -> EndpointConfig {
    EndpointConfig {
        silence_ms: cfg.audio.silence_ms,
        no_speech_timeout_ms: cfg.audio.no_speech_timeout_ms,
        max_capture_ms: cfg.audio.max_capture_ms,
        ..EndpointConfig::default()
    }
}

fn language_from(cfg: &Config) -> Language {
    Language::try_from(cfg.transcription.language.clone()).unwrap_or_else(|_| Language::auto())
}

fn parse_id(s: &str) -> Result<SessionId, String> {
    s.parse().map_err(|_| "STALE_REQUEST".to_string())
}

#[tauri::command]
fn get_snapshot(app: AppHandle, state: State<'_, AppState>) -> Snapshot {
    let ready = readiness(&state);
    state.handle.send(Command::SetReady(ready));
    let core = state.handle.snapshot(IPC_TIMEOUT);
    let mic_test_active = state
        .mic_test
        .lock()
        .map(|m| m.as_ref().is_some_and(|t| !t.is_finished()))
        .unwrap_or(false);
    let settings_revision = state
        .store
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(|s| s.get().revision))
        .unwrap_or(0);
    let name = |s: &Mutex<Option<Shortcut>>| {
        s.lock()
            .ok()
            .and_then(|s| s.map(|s| s.to_string()))
            .unwrap_or_default()
    };
    Snapshot {
        schema_version: 1,
        app_version: app.package_info().version.to_string(),
        platform: std::env::consts::OS,
        phase: core.as_ref().map(|c| c.phase),
        draft: core
            .as_ref()
            .and_then(|c| c.draft.as_ref())
            .map(|d| DraftMeta {
                id: d.id.to_string(),
                revision: d.revision,
                attempted: d.attempted,
            }),
        ready_error: ready.err(),
        accessibility: format!("{:?}", dictation_platform::accessibility_permission()),
        worker_found: selftest::worker_path().is_some(),
        active_model: models::active_model(&state).map(|m| m.id),
        settings_revision,
        settings_note: state.config_note.clone(),
        shortcuts: ShortcutStatus {
            toggle: name(&state.toggle),
            cancel: name(&state.cancel),
            error: state.shortcut_error.lock().ok().and_then(|e| e.clone()),
        },
        mic_test_active,
        last_probe: state.last_probe.lock().ok().and_then(|p| p.clone()),
    }
}

#[tauri::command]
fn start_dictation(state: State<'_, AppState>) -> Result<(), String> {
    let busy_test = state
        .mic_test
        .lock()
        .map(|m| m.as_ref().is_some_and(|t| !t.is_finished()))
        .unwrap_or(false);
    if busy_test {
        return Err("BUSY".into());
    }
    match state.handle.snapshot(IPC_TIMEOUT).map(|s| s.phase) {
        Some(Phase::Idle | Phase::DraftReady) => {
            state.handle.send(Command::Toggle);
            Ok(())
        }
        _ => Err("BUSY".into()),
    }
}

#[tauri::command]
fn stop_dictation(state: State<'_, AppState>) {
    state.handle.send(Command::Stop);
}

#[tauri::command]
fn cancel_dictation(state: State<'_, AppState>) {
    state.handle.send(Command::Cancel);
}

/// Settings/review window only (capability-scoped). Opening review revokes
/// automatic delivery for an in-flight session (docs/04 invariant 9).
#[tauri::command]
fn get_draft(state: State<'_, AppState>) -> Option<DraftView> {
    state.handle.send(Command::ReviewOpened);
    state.handle.draft(IPC_TIMEOUT).map(|d| DraftView {
        id: d.id.to_string(),
        revision: d.revision,
        text: d.text,
        reason: d.reason,
        attempted: d.attempted,
    })
}

#[tauri::command]
fn edit_draft(
    state: State<'_, AppState>,
    id: String,
    expected_revision: u32,
    text: String,
) -> Result<(), String> {
    state.handle.send(Command::DraftEdit {
        id: parse_id(&id)?,
        expected_revision,
        text,
    });
    Ok(())
}

#[tauri::command]
fn copy_draft(state: State<'_, AppState>, id: String, revision: u32) -> Result<(), String> {
    state.handle.send(Command::DraftCopy {
        id: parse_id(&id)?,
        revision,
    });
    Ok(())
}

#[tauri::command]
fn discard_draft(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state
        .handle
        .send(Command::DraftDiscard { id: parse_id(&id)? });
    Ok(())
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Result<Config, String> {
    let store = state.store.lock().map_err(|_| "state poisoned")?;
    store
        .as_ref()
        .map(|s| s.get().clone())
        .ok_or_else(|| "settings unavailable".into())
}

/// Validated merge-patch with optimistic concurrency; applies to the next capture.
#[tauri::command]
fn update_settings(
    state: State<'_, AppState>,
    expected_revision: u64,
    patch: serde_json::Value,
) -> Result<Config, String> {
    let mut store = state.store.lock().map_err(|_| "state poisoned")?;
    let store = store.as_mut().ok_or("settings unavailable")?;
    let cfg = store
        .update(expected_revision, &patch)
        .map_err(|e| e.to_string())?
        .clone();
    if let Ok(mut c) = state.capture_settings.lock() {
        *c = CaptureSettings {
            device_id: cfg.audio.device_id.clone(),
            endpoint: endpoint_from(&cfg),
        };
    }
    if let Ok(mut l) = state.language.lock() {
        *l = language_from(&cfg);
    }
    Ok(cfg)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceView {
    id: String,
    name: String,
    is_default: bool,
}

#[tauri::command]
fn list_input_devices() -> Result<Vec<DeviceView>, String> {
    dictation_audio::capture::list_input_devices()
        .map(|v| {
            v.into_iter()
                .map(|d| DeviceView {
                    id: d.id,
                    name: d.name,
                    is_default: d.is_default,
                })
                .collect()
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn start_mic_test(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: Option<String>,
) -> Result<(), String> {
    // Exclusive: never in parallel with dictation (docs/04 §3).
    if !matches!(
        state.handle.snapshot(IPC_TIMEOUT).map(|s| s.phase),
        Some(Phase::Idle | Phase::DraftReady)
    ) {
        return Err("BUSY".into());
    }
    let mut slot = state.mic_test.lock().map_err(|_| "state poisoned")?;
    if slot.as_ref().is_some_and(|t| !t.is_finished()) {
        return Err("BUSY".into());
    }
    if let Some(id) = &device_id {
        let known = dictation_audio::capture::list_input_devices().map_err(|e| e.to_string())?;
        if !known.iter().any(|d| &d.id == id) {
            return Err("unknown device".into());
        }
    }
    *slot = Some(MicTest::start(app, device_id)?);
    Ok(())
}

#[tauri::command]
fn stop_mic_test(state: State<'_, AppState>) {
    if let Ok(mut slot) = state.mic_test.lock()
        && let Some(mut t) = slot.take()
    {
        t.stop();
    }
}

#[tauri::command]
async fn run_worker_selftest(
    state: State<'_, AppState>,
) -> Result<selftest::SelftestReport, String> {
    let m = models::active_model(&state).ok_or("MODEL_UNAVAILABLE")?;
    let path = state.models.path(m);
    tauri::async_runtime::spawn_blocking(move || selftest::run(m, &path))
        .await
        .map_err(|e| e.to_string())?
}

/// M0 T0.4/T0.5 diagnostic kept for the Mac checklist: compare the frontmost
/// target before the toggle and shortly after the HUD appeared.
fn run_hud_probe(app: AppHandle) {
    std::thread::spawn(move || {
        let before = dictation_platform::capture_target();
        let before_s = format!("{before:?}");
        std::thread::sleep(Duration::from_millis(400));
        let check = match &before {
            Ok(s) => dictation_platform::check_target(s),
            Err(_) => dictation_platform::TargetCheck::Unknown,
        };
        let report = ProbeReport {
            before: before_s,
            after_hud_check: format!("{check:?}"),
            auto_delivery_would_be_allowed: check.allows_auto_delivery(),
        };
        if let Some(state) = app.try_state::<AppState>()
            && let Ok(mut p) = state.last_probe.lock()
        {
            *p = Some(report.clone());
        }
        let _ = app.emit_to("main", "probe_report", report);
    });
}

fn on_shortcut(app: &AppHandle, shortcut: &Shortcut, pressed: bool) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let is = |slot: &Mutex<Option<Shortcut>>| {
        slot.lock()
            .ok()
            .and_then(|s| *s)
            .is_some_and(|s| &s == shortcut)
    };
    if is(&state.toggle)
        && state
            .toggle_filter
            .lock()
            .is_ok_and(|mut f| f.on_event(pressed))
    {
        let idle = matches!(
            state.handle.snapshot(IPC_TIMEOUT).map(|s| s.phase),
            Some(Phase::Idle)
        );
        if idle {
            run_hud_probe(app.clone());
        }
        state.handle.send(Command::Toggle);
    } else if is(&state.cancel)
        && state
            .cancel_filter
            .lock()
            .is_ok_and(|mut f| f.on_event(pressed))
    {
        state.handle.send(Command::Cancel);
    }
}

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus(); // explicit user action from the tray
    }
}

fn quit(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut slot) = state.mic_test.lock()
            && let Some(mut t) = slot.take()
        {
            t.stop();
        }
        // Cancels any session, closes the microphone and kills the worker.
        if let Ok(mut c) = state.coordinator.lock()
            && let Some(c) = c.take()
        {
            c.shutdown();
        }
    }
    app.exit(0);
}

fn register_shortcuts(app: &AppHandle, cfg: &Config) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let gs = app.global_shortcut();
    let mut errors = vec![];
    for (chord, slot) in [
        (&cfg.shortcuts.toggle, &state.toggle),
        (&cfg.shortcuts.cancel, &state.cancel),
    ] {
        match chord.parse::<Shortcut>() {
            Ok(s) => match gs.register(s) {
                Ok(()) => {
                    if let Ok(mut slot) = slot.lock() {
                        *slot = Some(s);
                    }
                }
                Err(e) => errors.push(format!("SHORTCUT_CONFLICT {chord}: {e}")),
            },
            Err(e) => errors.push(format!("invalid shortcut {chord}: {e}")),
        }
    }
    if !errors.is_empty()
        && let Ok(mut slot) = state.shortcut_error.lock()
    {
        *slot = Some(errors.join("; "));
    }
}

#[allow(clippy::expect_used)] // startup failure is unrecoverable and must be visible
pub fn run() {
    tauri::Builder::default()
        // Single instance prevents duplicate shortcut and microphone ownership.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_settings(app)))
        // Rust-only use (model import picker); no dialog permission is granted to webviews.
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| on_shortcut(app, shortcut, event.state == ShortcutState::Pressed))
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            let (store, note) = match ConfigStore::open(&config_dir) {
                Ok((s, LoadOutcome::Loaded | LoadOutcome::CreatedDefault)) => (Some(s), None),
                Ok((s, LoadOutcome::RecoveredFromCorrupt { backup })) => {
                    (Some(s), Some(format!("Settings were unreadable and were reset; the old file is at {}", backup.display())))
                }
                Ok((s, LoadOutcome::FutureSchema { version })) => (
                    Some(s),
                    Some(format!("Settings were written by a newer version (schema {version}); using defaults without saving")),
                ),
                Err(e) => (None, Some(format!("Settings unavailable: {e}"))),
            };
            let cfg = store.as_ref().map(|s| s.get().clone()).unwrap_or_default();
            let models = ModelStore::open(&data_dir.join("models")).map_err(|e| e.to_string())?;
            // Restore the configured model if it is still installed; the worker
            // re-verifies its hash while loading.
            let initial = dictation_models::find(&cfg.transcription.model_id)
                .and_then(|m| models.installed_path(m).map(|path| ModelRef { path, sha256: m.sha256.to_string() }));
            let model_ref = Arc::new(Mutex::new(initial));

            let capture_settings: SharedCaptureSettings = Arc::new(Mutex::new(CaptureSettings {
                device_id: cfg.audio.device_id.clone(),
                endpoint: endpoint_from(&cfg),
            }));
            let language = Arc::new(Mutex::new(language_from(&cfg)));
            let transcriber = WorkerTranscriber::new(
                WorkerConfig {
                    exe: selftest::worker_path().unwrap_or_default(),
                    model: Arc::clone(&model_ref),
                    language: Arc::clone(&language),
                    job_timeout: Duration::from_secs(60),
                },
                1,
            );
            let coordinator = Coordinator::spawn(
                Adapters {
                    capture: Box::new(DspCapture::new(
                        live_source(),
                        Arc::clone(&capture_settings),
                        DetectorConfig::default(),
                        Duration::from_millis(10),
                    )),
                    transcriber: Box::new(transcriber),
                    delivery: Box::new(CopyOnlyDelivery),
                    notifier: Box::new(TauriNotifier { app: app.handle().clone() }),
                },
                1,
            );
            let handle = coordinator.handle();

            app.manage(AppState {
                coordinator: Mutex::new(Some(coordinator)),
                handle,
                store: Mutex::new(store),
                models,
                model_ref,
                model_ops: Mutex::new(HashMap::new()),
                config_note: note,
                capture_settings,
                language,
                toggle: Mutex::new(None),
                cancel: Mutex::new(None),
                toggle_filter: Mutex::new(ChordFilter::default()),
                cancel_filter: Mutex::new(ChordFilter::default()),
                shortcut_error: Mutex::new(None),
                mic_test: Mutex::new(None),
                last_probe: Mutex::new(None),
            });
            refresh_readiness(&app.state::<AppState>());
            register_shortcuts(app.handle(), &cfg);

            let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let cancel = MenuItem::with_id(app, "cancel", "Cancel dictation", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Local Dictation", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings, &cancel, &quit_item])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .tooltip("Local Dictation")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "settings" => show_settings(app),
                    "cancel" => {
                        if let Some(s) = app.try_state::<AppState>() {
                            s.handle.send(Command::Cancel);
                        }
                    }
                    "quit" => quit(app),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing settings hides it; Quit is explicit from the tray.
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            start_dictation,
            stop_dictation,
            cancel_dictation,
            get_draft,
            edit_draft,
            copy_draft,
            discard_draft,
            get_settings,
            update_settings,
            list_input_devices,
            start_mic_test,
            stop_mic_test,
            run_worker_selftest,
            models::get_models,
            models::install_model,
            models::import_model,
            models::verify_model,
            models::cancel_model_operation,
            models::remove_model,
            models::activate_model,
            models::unload_model
        ])
        .run(tauri::generate_context!())
        .expect("error while running Local Dictation");
}
