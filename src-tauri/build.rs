#[allow(clippy::panic)] // build scripts report failure by panicking
fn main() {
    // Every app command gets an explicit permission so capabilities can scope
    // them per window (the HUD gets none; docs/06 §4).
    let manifest = tauri_build::AppManifest::new().commands(&[
        "get_snapshot",
        "start_dictation",
        "stop_dictation",
        "cancel_dictation",
        "get_draft",
        "edit_draft",
        "copy_draft",
        "discard_draft",
        "get_settings",
        "update_settings",
        "list_input_devices",
        "start_mic_test",
        "stop_mic_test",
        "run_worker_selftest",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .unwrap_or_else(|e| panic!("tauri-build failed: {e:#}"));
}
