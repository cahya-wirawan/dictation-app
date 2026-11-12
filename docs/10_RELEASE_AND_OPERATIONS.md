# 10 — Packaging, release and support

## 1. Build outputs

- macOS: signed/notarized `.app` distributed in DMG; worker signed as nested executable and verified with the whole bundle. Initial architecture arm64. Intel/universal is separately qualified.
- Windows: signed native x64 installer with documented WebView2 prerequisite/offline policy and bundled target-specific worker. Test installation as a standard user.
- Linux: distribution-specific package experiments only after native dependencies and desktop support are known. An AppImage is not proof of universal desktop integration.

Tauri supports platform-specific bundles and documents target-specific sidecar naming and packaging [S5, S13]. Use native build runners/toolchains and test the actual packaged executable path. Never discover the engine through shell PATH or download a replacement executable at runtime.

## 2. Reproducible release inputs

Record Rust toolchain, Node/package manager, Cargo/frontend lockfiles, Tauri/plugin versions, whisper.cpp revision and any submodules, FFI generator revision, C/C++ compiler/SDK, build flags and model manifest hashes. Emit an SBOM and third-party notices for app, worker, native dependencies and model assets. Local inference software and model weights may have distinct licenses; verify both.

Keep signing credentials in the release environment secret store. Pull-request builds must not receive production signing secrets. Pin CI actions by immutable revision where possible; review dependency updates and engine parser fixes promptly. Do not fabricate a reproducible-build claim unless independent build comparison has been performed.

## 3. Pre-release checklist

- [ ] Unit, fixture, protocol, UI and native test gates pass.
- [ ] Signed packaged worker starts from installed resource path and exits with parent.
- [ ] Clean-machine microphone/Accessibility prompt and denied/revoked states work.
- [ ] Model import works offline; download tamper/partial/cancel tests pass.
- [ ] Supported app versions and target-element behavior are listed.
- [ ] CPU baseline and optional accelerator profiles have independent benchmarks.
- [ ] No remote script/font requirement, open listener or inference egress exists.
- [ ] Diagnostics contain no dictated text or audio.
- [ ] Permissions survive a normal upgrade or onboarding explains required regrant.
- [ ] Install, upgrade, downgrade warning and uninstall paths are documented.
- [ ] Release notes name limitations, models, engine version and minimum OS.

## 4. Update and migration policy

v1 uses a user-initiated release-page/download flow. Keep automatic updater out until signed-update policy is implemented and tested. App upgrades must preserve configuration and compatible verified models. Version configuration schema independently of application semantic version. Newer config opened by an older app produces a safe warning and separate defaults, not silent data loss.

Worker protocol version is an exact compatibility boundary; a mismatched worker must not run inference. App and worker ship together. Model manifest maps compatible formats/revisions; incompatible models remain available for removal/export but are not loaded.

Rollback means reinstalling a previously signed app with compatible model/config data, not fetching arbitrary engine binaries. Preserve a pre-migration config backup. Never delete model files merely because the active model load failed.

## 5. Support diagnostics

A user can export redacted version/build/backend/model IDs, timing summaries, permission codes and recent error codes. No automatic uploads. Include a preview and explicit Save action. Support docs explain how to select a mic, choose language/model, adjust silence, grant permissions, resolve shortcut conflicts and use Copy when an app is unsupported.

If transcription hangs, cancel stops input eligibility, kills/reaps the worker and returns to ready after restart. If audio devices change, stop recording and require selection. If clipboard is busy, retain text and ask for explicit copy/retry. If paste was dispatched but not observed, advise checking the target first; do not automatically resend.

## 6. Uninstall and data explanation

Uninstalling removes the app bundle/installer-owned binaries. Provide a documented optional removal of app settings/models/log directories. The app cannot guarantee deletion of OS clipboard history, sync copies, OS logs or swap. A Settings “Clear draft” operation affects only the app's in-memory draft. This distinction belongs in user-facing privacy help.
