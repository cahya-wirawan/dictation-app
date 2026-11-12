//! Tauri shell for the M0 feasibility spike.
//!
//! What exists here: tray + explicit Quit, toggle/cancel global chords with
//! repeat filtering, a non-activating HUD with a before/after target identity
//! probe, an exclusive mic test with a level meter, and an in-app worker
//! self-test. There is deliberately no command that pastes, types, runs a
//! process, reads an arbitrary path or downloads a URL. The full coordinator
//! (docs/03 §1) arrives in M1.

mod mictest;
mod selftest;

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use dictation_domain::chord::ChordFilter;
use mictest::MicTest;

const TOGGLE: &str = "ctrl+alt+space";
const CANCEL: &str = "ctrl+alt+escape";

#[derive(Default)]
struct AppState {
    mic_test: Mutex<Option<MicTest>>,
    toggle_filter: Mutex<ChordFilter>,
    cancel_filter: Mutex<ChordFilter>,
    shortcut_error: Mutex<Option<String>>,
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
struct ShortcutStatus {
    toggle: &'static str,
    cancel: &'static str,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    schema_version: u32,
    app_version: String,
    platform: &'static str,
    accessibility: String,
    worker_found: bool,
    model_installed: bool,
    model_path: String,
    model_source_url: &'static str,
    shortcuts: ShortcutStatus,
    mic_test_active: bool,
    last_probe: Option<ProbeReport>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HudPhase {
    phase: &'static str,
}

fn model_path(app: &AppHandle) -> std::path::PathBuf {
    let dir = app.path().app_data_dir().unwrap_or_default();
    selftest::model_path(&dir)
}

#[tauri::command]
fn get_snapshot(app: AppHandle, state: State<'_, AppState>) -> Snapshot {
    let model = model_path(&app);
    let mic_test_active = state
        .mic_test
        .lock()
        .map(|m| m.as_ref().is_some_and(|t| !t.is_finished()))
        .unwrap_or(false);
    Snapshot {
        schema_version: 1,
        app_version: app.package_info().version.to_string(),
        platform: std::env::consts::OS,
        accessibility: format!("{:?}", dictation_platform::accessibility_permission()),
        worker_found: selftest::worker_path().is_some(),
        model_installed: model.is_file(),
        model_path: model.display().to_string(),
        model_source_url: selftest::BASE_MULTILINGUAL.url,
        shortcuts: ShortcutStatus {
            toggle: TOGGLE,
            cancel: CANCEL,
            error: state.shortcut_error.lock().ok().and_then(|e| e.clone()),
        },
        mic_test_active,
        last_probe: state.last_probe.lock().ok().and_then(|p| p.clone()),
    }
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
    let mut slot = state.mic_test.lock().map_err(|_| "state poisoned")?;
    if slot.as_ref().is_some_and(|t| !t.is_finished()) {
        return Err("BUSY".into());
    }
    if let Some(id) = &device_id {
        // Only ids the backend itself enumerated are accepted.
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
async fn run_worker_selftest(app: AppHandle) -> Result<selftest::SelftestReport, String> {
    let model = model_path(&app);
    tauri::async_runtime::spawn_blocking(move || selftest::run(&model))
        .await
        .map_err(|e| e.to_string())?
}

/// M0 T0.4/T0.5: capture the frontmost target, show the HUD, then verify the
/// target (app, window, focused element) is unchanged.
fn run_hud_probe(app: AppHandle) {
    std::thread::spawn(move || {
        let before = dictation_platform::capture_target();
        let before_s = format!("{before:?}");
        if let Some(hud) = app.get_webview_window("hud") {
            let _ = hud.show();
        }
        let _ = app.emit_to("hud", "phase_changed", HudPhase { phase: "listening" });
        std::thread::sleep(Duration::from_millis(400));
        let check = match &before {
            Ok(s) => dictation_platform::check_target(s),
            Err(_) => dictation_platform::TargetCheck::Unknown,
        };
        std::thread::sleep(Duration::from_millis(1200));
        let _ = app.emit_to("hud", "phase_changed", HudPhase { phase: "idle" });
        if let Some(hud) = app.get_webview_window("hud") {
            let _ = hud.hide();
        }
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
    let is_toggle = TOGGLE.parse::<Shortcut>().is_ok_and(|s| &s == shortcut);
    let is_cancel = CANCEL.parse::<Shortcut>().is_ok_and(|s| &s == shortcut);
    if is_toggle
        && state
            .toggle_filter
            .lock()
            .is_ok_and(|mut f| f.on_event(pressed))
    {
        run_hud_probe(app.clone());
    } else if is_cancel
        && state
            .cancel_filter
            .lock()
            .is_ok_and(|mut f| f.on_event(pressed))
    {
        let _ = app.emit_to("hud", "phase_changed", HudPhase { phase: "idle" });
        if let Some(hud) = app.get_webview_window("hud") {
            let _ = hud.hide();
        }
    }
}

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus(); // explicit user action from the tray
    }
}

fn quit(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>()
        && let Ok(mut slot) = state.mic_test.lock()
        && let Some(mut t) = slot.take()
    {
        t.stop();
    }
    app.exit(0);
}

#[allow(clippy::expect_used)] // startup failure is unrecoverable and must be visible
pub fn run() {
    tauri::Builder::default()
        // Single instance prevents duplicate shortcut and microphone ownership.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_settings(app)
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    on_shortcut(app, shortcut, event.state == ShortcutState::Pressed)
                })
                .build(),
        )
        .manage(AppState::default())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let quit_item =
                MenuItem::with_id(app, "quit", "Quit Local Dictation", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings, &quit_item])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .tooltip("Local Dictation")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "settings" => show_settings(app),
                    "quit" => quit(app),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            let gs = app.global_shortcut();
            let mut errors = vec![];
            for chord in [TOGGLE, CANCEL] {
                if let Err(e) = gs.register(chord) {
                    errors.push(format!("SHORTCUT_CONFLICT {chord}: {e}"));
                }
            }
            if !errors.is_empty()
                && let Ok(mut slot) = app.state::<AppState>().shortcut_error.lock()
            {
                *slot = Some(errors.join("; "));
            }
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
            list_input_devices,
            start_mic_test,
            stop_mic_test,
            run_worker_selftest
        ])
        .run(tauri::generate_context!())
        .expect("error while running Local Dictation");
}
