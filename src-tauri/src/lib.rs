//! Tauri shell (M1): tray, global shortcuts, HUD, settings window and the IPC
//! surface over the Rust coordinator.
//!
//! The coordinator (crates/coordinator) owns every session decision. This
//! file only translates OS/UI events into coordinator commands and coordinator
//! notifications into scoped window events. There is deliberately no command
//! that pastes, types, runs a process, reads an arbitrary path or downloads a
//! URL. Automatic delivery runs through `PlatformDelivery` (crates/coordinator):
//! one validated clipboard + paste transaction per session, or a memory-only
//! draft when the target cannot be validated.

mod diagnostics;
mod mictest;
mod models;
mod onboarding;
mod selftest;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dictation_audio::detector::DetectorConfig;
use dictation_audio::endpoint::EndpointConfig;
use dictation_coordinator::actor::ModelRef;
use dictation_coordinator::actor::{Adapters, Command, Coordinator, CoordinatorHandle, Notifier};
use dictation_coordinator::adapters::{
    CaptureSettings, DspCapture, SharedCaptureSettings, WorkerConfig, WorkerTranscriber,
    live_source,
};
use dictation_coordinator::config_store::{ConfigStore, LoadOutcome};
use dictation_coordinator::delivery::{NativePlatform, PlatformDelivery, SharedExclusions};
use dictation_coordinator::diagnostics::{Diagnostics, RecordingNotifier};
use dictation_domain::chord::ChordFilter;
use dictation_domain::config::Config;
use dictation_domain::config::DeliveryModeSetting;
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
    paste_draft: Mutex<Option<Shortcut>>,
    toggle_filter: Mutex<ChordFilter>,
    cancel_filter: Mutex<ChordFilter>,
    paste_filter: Mutex<ChordFilter>,
    /// Filtered shortcut presses for the `shortcuts` thread.
    shortcut_tx: Mutex<std::sync::mpsc::Sender<ShortcutAction>>,
    excluded: SharedExclusions,
    shortcut_error: Mutex<Option<String>>,
    mic_test: Mutex<Option<MicTest>>,
    last_probe: Mutex<Option<ProbeReport>>,
    pub(crate) diagnostics: Arc<Diagnostics>,
    session: Option<SessionView>,
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
    expires_in_ms: u64,
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
    onboarding_complete: bool,
    shortcuts: ShortcutStatus,
    mic_test_active: bool,
    last_probe: Option<ProbeReport>,
    /// Linux only: what this desktop session supports (probed once at start).
    session: Option<SessionView>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionView {
    kind: &'static str,
    desktop: Option<String>,
    global_shortcuts_reliable: bool,
    copy_available: bool,
    auto_paste: bool,
}

#[cfg(target_os = "linux")]
fn probe_session() -> Option<SessionView> {
    use dictation_platform::SessionKind;
    let p = dictation_platform::probe();
    Some(SessionView {
        kind: match p.kind {
            SessionKind::X11 => "x11",
            SessionKind::Wayland => "wayland",
            SessionKind::None => "none",
        },
        desktop: p.desktop,
        global_shortcuts_reliable: p.global_shortcuts_reliable,
        copy_available: p.x11_reachable,
        auto_paste: false,
    })
}

#[cfg(not(target_os = "linux"))]
fn probe_session() -> Option<SessionView> {
    None
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
    expires_in_ms: u64,
}

/// Draft text is readable only from the settings and review windows. The
/// capability files enforce this too; Rust checks the label as well
/// (docs/06 §1: capabilities do not sandbox mistakes in Rust).
fn draft_window(w: &tauri::WebviewWindow) -> Result<(), String> {
    match w.label() {
        "main" | "review" => Ok(()),
        _ => Err("not permitted from this window".into()),
    }
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
    /// Bumped on every phase/banner change so a delayed hide never hides
    /// the HUD of a newer session.
    hud_seq: Arc<std::sync::atomic::AtomicU64>,
}

/// Outcomes worth a short HUD banner when the app has no visible window
/// (docs/02 §1 "Draft banner"). The banner never carries text.
fn banner_for(o: &Outcome) -> bool {
    matches!(
        o,
        Outcome::DraftReady(_)
            | Outcome::Failed(_)
            | Outcome::NoSpeech
            | Outcome::NoUsableText
            | Outcome::CancelTooLate
    )
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
        self.hud_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        update_tray_status(&self.app, Some(phase));
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
        let _ = self
            .app
            .emit_to("review", "outcome", OutcomePayload { outcome });
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
        if !show_hud || !banner_for(&outcome) {
            return;
        }
        let _ = self
            .app
            .emit_to("hud", "hud_banner", OutcomePayload { outcome });
        let seq = self
            .hud_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        if let Some(hud) = self.app.get_webview_window("hud") {
            let _ = hud.show();
        }
        let (app, hud_seq) = (self.app.clone(), Arc::clone(&self.hud_seq));
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(3500));
            if hud_seq.load(std::sync::atomic::Ordering::SeqCst) == seq
                && let Some(hud) = app.get_webview_window("hud")
            {
                let _ = hud.hide();
            }
        });
    }

    fn level(&mut self, level: f32) {
        let _ = self.app.emit_to("hud", "mic_level", LevelPayload { level });
    }
}

/// Readiness for new sessions: worker binary present and an active model
/// that is still installed with the approved size.
pub(crate) fn readiness(state: &AppState) -> Result<(), AppError> {
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

/// Tray tooltip mirrors the coordinator phase and readiness, so the state is
/// visible without opening a window (docs/02 §1). Never contains text.
///
/// Called from the coordinator thread. Tauri's tray setters wait for the main
/// thread to run them, and sync commands make the main thread wait for the
/// coordinator, so the update is queued to the main thread, never awaited.
pub(crate) fn update_tray_status(app: &AppHandle, phase: Option<Phase>) {
    let ready = app.try_state::<AppState>().map(|s| readiness(&s));
    let status = match phase {
        Some(Phase::Starting | Phase::Listening) => "Listening…",
        Some(Phase::Finalizing | Phase::Transcribing) => "Transcribing…",
        Some(Phase::Delivering) => "Pasting…",
        Some(Phase::DraftReady) => "Text ready to review",
        Some(Phase::Recovering) => "Recovering…",
        Some(Phase::Idle) | None => match ready {
            Some(Ok(())) => "Ready",
            Some(Err(AppError::ModelUnavailable)) => "Setup needed: install a model",
            Some(Err(AppError::ModelInvalid)) => "Model needs attention",
            _ => "Not ready",
        },
    };
    let tooltip = format!("Local Dictation — {status}");
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = handle.tray_by_id("main-tray") {
            let _ = tray.set_tooltip(Some(tooltip));
        }
    });
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

// Commands that wait for a coordinator reply are `async` so they run on the
// async runtime: a sync command runs on the main thread and would freeze the
// whole app (windows, tray, AX replies) while it waits.
#[tauri::command]
async fn get_snapshot(app: AppHandle, state: State<'_, AppState>) -> Result<Snapshot, String> {
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
    Ok(Snapshot {
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
                expires_in_ms: d.expires_in_ms,
            }),
        ready_error: ready.err(),
        accessibility: format!("{:?}", dictation_platform::accessibility_permission()),
        worker_found: selftest::worker_path().is_some(),
        active_model: models::active_model(&state).map(|m| m.id),
        settings_revision,
        settings_note: state.config_note.clone(),
        onboarding_complete: state
            .store
            .lock()
            .ok()
            .and_then(|s| s.as_ref().map(|s| s.get().ui.onboarding_complete))
            .unwrap_or(false),
        shortcuts: ShortcutStatus {
            toggle: name(&state.toggle),
            cancel: name(&state.cancel),
            error: state.shortcut_error.lock().ok().and_then(|e| e.clone()),
        },
        mic_test_active,
        last_probe: state.last_probe.lock().ok().and_then(|p| p.clone()),
        session: state.session.clone(),
    })
}

#[tauri::command]
async fn start_dictation(state: State<'_, AppState>) -> Result<(), String> {
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
async fn get_draft(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<DraftView>, String> {
    draft_window(&window)?;
    state.handle.send(Command::ReviewOpened);
    let expires_in_ms = state
        .handle
        .snapshot(IPC_TIMEOUT)
        .and_then(|s| s.draft)
        .map_or(0, |d| d.expires_in_ms);
    Ok(state.handle.draft(IPC_TIMEOUT).map(|d| DraftView {
        id: d.id.to_string(),
        revision: d.revision,
        text: d.text,
        reason: d.reason,
        attempted: d.attempted,
        expires_in_ms,
    }))
}

#[tauri::command]
fn edit_draft(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    id: String,
    expected_revision: u32,
    text: String,
) -> Result<(), String> {
    draft_window(&window)?;
    state.handle.send(Command::DraftEdit {
        id: parse_id(&id)?,
        expected_revision,
        text,
    });
    Ok(())
}

#[tauri::command]
fn copy_draft(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    id: String,
    revision: u32,
) -> Result<(), String> {
    draft_window(&window)?;
    state.handle.send(Command::DraftCopy {
        id: parse_id(&id)?,
        revision,
    });
    Ok(())
}

#[tauri::command]
fn discard_draft(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    draft_window(&window)?;
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
    app: AppHandle,
    state: State<'_, AppState>,
    expected_revision: u64,
    patch: serde_json::Value,
) -> Result<Config, String> {
    let (cfg, shortcuts_changed) = {
        let mut store = state.store.lock().map_err(|_| "state poisoned")?;
        let store = store.as_mut().ok_or("settings unavailable")?;
        let before = store.get().shortcuts.clone();
        let cfg = store
            .update(expected_revision, &patch)
            .map_err(|e| e.to_string())?
            .clone();
        let changed = cfg.shortcuts != before;
        (cfg, changed)
    };
    apply_delivery_settings(&state, &cfg);
    if shortcuts_changed {
        register_shortcuts(&app, &cfg);
    }
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
async fn start_mic_test(
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

fn apply_delivery_settings(state: &AppState, cfg: &Config) {
    state.handle.send(Command::SetAutoDelivery(
        cfg.delivery.mode == DeliveryModeSetting::AutoPaste,
    ));
    state
        .handle
        .send(Command::SetContinuous(cfg.audio.continuous));
    if let Ok(mut e) = state.excluded.lock() {
        e.clone_from(&cfg.delivery.excluded_apps);
    }
}

/// A filtered shortcut press, handled on the `shortcuts` thread.
#[derive(Clone, Copy)]
enum ShortcutAction {
    Toggle,
    Cancel,
    PasteDraft,
}

/// Runs on the main thread (global-shortcut callback), so it only filters the
/// press and queues it. Anything that waits for the coordinator happens on
/// the `shortcuts` thread, which keeps presses in order.
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
    let hit = |filter: &Mutex<ChordFilter>| filter.lock().is_ok_and(|mut f| f.on_event(pressed));
    let action = if is(&state.toggle) {
        hit(&state.toggle_filter).then_some(ShortcutAction::Toggle)
    } else if is(&state.cancel) {
        hit(&state.cancel_filter).then_some(ShortcutAction::Cancel)
    } else if is(&state.paste_draft) {
        hit(&state.paste_filter).then_some(ShortcutAction::PasteDraft)
    } else {
        None
    };
    if let Some(a) = action
        && let Ok(tx) = state.shortcut_tx.lock()
    {
        let _ = tx.send(a);
    }
}

fn run_shortcut(app: &AppHandle, action: ShortcutAction) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match action {
        ShortcutAction::Toggle => {
            let idle = matches!(
                state.handle.snapshot(IPC_TIMEOUT).map(|s| s.phase),
                Some(Phase::Idle)
            );
            if idle {
                run_hud_probe(app.clone());
            }
            state.handle.send(Command::Toggle);
        }
        ShortcutAction::Cancel => {
            state.handle.send(Command::Cancel);
        }
        ShortcutAction::PasteDraft => {
            // Explicit delivery: the target is the field focused at this press.
            if let Some(d) = state.handle.snapshot(IPC_TIMEOUT).and_then(|s| s.draft) {
                state.handle.send(Command::DraftPaste {
                    id: d.id,
                    revision: d.revision,
                });
            }
        }
    }
}

pub(crate) fn show_review(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state.handle.send(Command::ReviewOpened);
    }
    if let Some(w) = app.get_webview_window("review") {
        let _ = w.show();
        let _ = w.set_focus(); // deliberate: review takes focus by design
    }
}

pub(crate) fn show_settings(app: &AppHandle) {
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

/// (Re)registers the configured chords. Previously registered chords are
/// released first, so a changed setting never leaves a stale global shortcut.
fn register_shortcuts(app: &AppHandle, cfg: &Config) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let gs = app.global_shortcut();
    let mut errors = vec![];
    let wanted = [
        (Some(&cfg.shortcuts.toggle), &state.toggle),
        (Some(&cfg.shortcuts.cancel), &state.cancel),
        (cfg.shortcuts.paste_draft.as_ref(), &state.paste_draft),
    ];
    for (_, slot) in &wanted {
        if let Ok(mut slot) = slot.lock()
            && let Some(old) = slot.take()
        {
            let _ = gs.unregister(old);
        }
    }
    for (chord, slot) in wanted {
        let Some(chord) = chord else { continue };
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
    if let Ok(mut slot) = state.shortcut_error.lock() {
        *slot = (!errors.is_empty()).then(|| errors.join("; "));
    }
}

#[allow(clippy::expect_used)] // startup failure is unrecoverable and must be visible
pub fn run() {
    tauri::Builder::default()
        // Single instance prevents duplicate shortcut and microphone ownership.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_settings(app)))
        // Rust-only use (model import picker); no dialog permission is granted to webviews.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
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
            let excluded: SharedExclusions = Arc::new(Mutex::new(cfg.delivery.excluded_apps.clone()));
            let diagnostics = Diagnostics::new();
            let transcriber = WorkerTranscriber::new(
                WorkerConfig {
                    exe: selftest::worker_path().unwrap_or_default(),
                    model: Arc::clone(&model_ref),
                    language: Arc::clone(&language),
                    job_timeout: Duration::from_secs(60),
                },
                1,
            )
            .with_diagnostics(Arc::clone(&diagnostics));
            let coordinator = Coordinator::spawn(
                Adapters {
                    capture: Box::new(DspCapture::new(
                        live_source(),
                        Arc::clone(&capture_settings),
                        DetectorConfig::default(),
                        Duration::from_millis(10),
                    )),
                    transcriber: Box::new(transcriber),
                    delivery: Box::new(PlatformDelivery::spawn(|| NativePlatform, Arc::clone(&excluded))),
                    notifier: Box::new(RecordingNotifier {
                        inner: TauriNotifier {
                            app: app.handle().clone(),
                            hud_seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                        },
                        diagnostics: Arc::clone(&diagnostics),
                    }),
                },
                1,
            );
            let handle = coordinator.handle();

            // Sleep, screen lock, user switch and logout cancel any session and
            // clear the draft; nothing resumes on wake (docs/04 invariant 10).
            let suspend_handle = handle.clone();
            let suspend_app = app.handle().clone();
            let watch = dictation_platform::watch_suspend(Arc::new(move || {
                suspend_handle.send(Command::Suspend);
                if let Some(s) = suspend_app.try_state::<AppState>()
                    && let Ok(mut slot) = s.mic_test.lock()
                    && let Some(mut t) = slot.take()
                {
                    t.stop();
                }
            }));
            // Observers live for the whole process (main-thread objects).
            let _watch: &'static mut _ = Box::leak(Box::new(watch));

            let (shortcut_tx, shortcut_rx) = std::sync::mpsc::channel();
            let shortcut_app = app.handle().clone();
            std::thread::Builder::new()
                .name("shortcuts".into())
                .spawn(move || {
                    while let Ok(a) = shortcut_rx.recv() {
                        run_shortcut(&shortcut_app, a);
                    }
                })?;

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
                paste_draft: Mutex::new(None),
                toggle_filter: Mutex::new(ChordFilter::default()),
                cancel_filter: Mutex::new(ChordFilter::default()),
                paste_filter: Mutex::new(ChordFilter::default()),
                shortcut_tx: Mutex::new(shortcut_tx),
                excluded,
                shortcut_error: Mutex::new(None),
                mic_test: Mutex::new(None),
                last_probe: Mutex::new(None),
                diagnostics,
                session: probe_session(),
            });
            refresh_readiness(&app.state::<AppState>());
            apply_delivery_settings(&app.state::<AppState>(), &cfg);
            register_shortcuts(app.handle(), &cfg);

            let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let review = MenuItem::with_id(app, "review", "Review text…", true, None::<&str>)?;
            let cancel = MenuItem::with_id(app, "cancel", "Cancel dictation", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Local Dictation", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&review, &cancel, &settings, &quit_item])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .tooltip("Local Dictation")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "settings" => show_settings(app),
                    "review" => show_review(app),
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
            update_tray_status(app.handle(), None);
            onboarding::first_run_show(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing settings hides it; Quit is explicit from the tray.
            if let WindowEvent::CloseRequested { api, .. } = event
                && matches!(window.label(), "main" | "review")
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
            models::unload_model,
            onboarding::check_permissions,
            onboarding::request_permission,
            onboarding::open_privacy_settings,
            onboarding::set_shortcut,
            onboarding::pause_shortcuts,
            onboarding::resume_shortcuts,
            onboarding::complete_onboarding,
            onboarding::set_launch_at_login,
            onboarding::open_review,
            diagnostics::get_diagnostics,
            diagnostics::save_diagnostics
        ])
        .run(tauri::generate_context!())
        .expect("error while running Local Dictation");
}
