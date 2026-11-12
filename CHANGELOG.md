# Changelog

All notable changes to Local Dictation are documented here. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).

## Versioning policy

- **One version for the whole app.** It lives in `[workspace.package].version` in `Cargo.toml`. All crates inherit it, Tauri reads it (so `tauri.conf.json` has no `version`), and `package.json` mirrors it. `pnpm version:check` fails if any of them disagree.
- **Before 1.0.0**, MINOR marks a milestone or a user-visible behavior change, including any breaking change. PATCH is for fixes that change no contract. 1.0.0 is the first release that passes the macOS release gates in docs/08 §4.
- **Pre-releases** use `-alpha.N`, `-beta.N` and `-rc.N`, e.g. `0.2.0-rc.1`.
- **Separately versioned contracts:**
  - the worker wire protocol (`PROTOCOL_VERSION`)
  - the config file `schema_version`
  - the event envelope `schemaVersion`

  These change only when their format changes. Any such change is noted below.
- **Releasing:** add entries under `[Unreleased]`, then run `pnpm version:bump <major|minor|patch|X.Y.Z>`. That moves the entries into a dated section, updates every version and `Cargo.lock`, and prints the commit and tag commands. Tags are `vX.Y.Z`.

## [Unreleased]

## [0.4.0] - 2026-11-12

Milestone M3 (delivery vertical slice) is complete.

### Added

- **Automatic paste** into the app you were dictating into (docs/05):
  - The target is captured at the shortcut press and re-validated after transcription.
  - The app waits for you to release the shortcut keys, never releasing them for you.
  - It writes plain text to the clipboard, re-validates, then sends exactly one paste.
  - App or field switches, password fields, unknown targets, terminals and excluded apps fall back to a draft. The app never refocuses anything.
- **Explicit paste for unsent text:** an opt-in `Ctrl+Option+V` pastes the draft into whichever field is focused at that press.
- **Settings:** a toggle between automatic paste and review-and-copy.
- **New error code:** `MODIFIERS_HELD`, for when the shortcut keys are still held at paste time.

## [0.3.0] - 2026-11-12

Milestone M2 (local engine and models) is complete.

### Added

- **Model manager** (`dictation-models`):
  - A bundled manifest of Whisper Base and Small (multilingual), pinned to an immutable repo revision, with independently verified checksums.
  - Resumable HTTPS download with a host allowlist on every redirect, size limits, Content-Range validation and streaming SHA-256.
  - Offline import, verify and remove, all with private 0600 files.
- **Models in the app:** download (with progress, cancel and resume), import a file through a Rust-owned picker, verify, use, unload and remove. The first installed model becomes active automatically.
- **Safe model activation:** a new model is loaded in a fresh worker and smoke-tested before it replaces the current one. A failed load keeps the previous model.

### Changed

- **Single-pass verified model load:** the worker now loads whisper.cpp through a custom loader that hashes the same open, symlink-refusing file descriptor. This closes the hash-then-reopen window and halves model I/O.
- **Model location:** the app uses the configured model from its private `models` directory. The development-only `DICTATION_MODEL_PATH` override is removed.

## [0.2.0] - 2026-11-12

Milestone M1 (domain and capture) is complete: the app dictates into a copyable draft. Automatic paste is not enabled yet (M3). The M1 run on the Mac in a GUI session is still pending; see `docs/IMPLEMENTATION_STATUS.md`.

### Added

- **CI** (`.github/workflows/ci.yml`), with every action pinned to a commit SHA:
  - frontend typecheck, lint, build and version check;
  - Rust fmt, clippy `-D warnings` and tests (including real inference) on Ubuntu 24.04 and macOS 26 arm64;
  - an unsigned macOS `.app` artifact, with checks of the bundle layout.
- `scripts/fetch-model.sh`: downloads the approved model and verifies its size and SHA-256.
- **M1 session state machine** (`dictation-domain::session`): a pure reducer over the docs/04 phases.
  - Session id, generation and worker epoch fencing.
  - The first finish request wins, so a stop/endpoint race yields one job.
  - Cancel is serialized with the delivery commit, and the ledger crosses commit at most once.
  - A memory-only draft with revisions, a 10-minute TTL, copy, discard and explicit paste; review revokes automatic delivery, and suspend clears everything.
- **Configuration** (`dictation-domain::config`, `dictation-coordinator::config_store`):
  - Validated schema and merge-patch with revision conflicts.
  - Atomic, user-private settings file; corrupt files are backed up; future-schema files are left untouched.
- **DSP worker** (`dictation-audio::dsp`): drains the capture ring into the pipeline and closes the microphone before reporting any result.
- **Coordinator actor** with capture, worker and delivery adapters. Delivery is copy-only for now.
- **The app does dictation:**
  - The toggle shortcut records, finishes on silence or a second press, and transcribes locally.
  - The text appears as a draft in the settings window, ready to copy.
  - The tray has Cancel, the HUD shows phase and level, and settings (microphone, language, silence, no-speech timeout, HUD) apply to the next session.

### Fixed

- macOS: the worker failed to link because ggml uses Accelerate (vDSP); it now links `-framework Accelerate`.
- macOS: one deployment target (13.0) for rustc, cmake and cc, via `.cargo/config.toml`. Previously whisper.cpp objects were built for the SDK's macOS version.
- x86_64 portable builds (`DICTATION_GGML_NATIVE=OFF`) now target x86-64-v3 instead of SSE-only. This is about 10x faster.
- `--features metal` is a no-op on non-Apple targets.

## [0.1.0] - 2026-11-12

Milestone M0 feasibility spike, partially complete. See `docs/IMPLEMENTATION_STATUS.md` for what was verified, where, and what still needs a Mac.

### Added

- **Workspace and tooling:**
  - Cargo workspace on Rust 1.93.0 (pinned toolchain).
  - Tauri 2.12 shell.
  - Vite 8 + React 19 + strict TypeScript 6.0 frontend, with typescript-eslint `strictTypeChecked`.
- **`dictation-worker`:** a bundled native inference process that statically links whisper.cpp v1.9.5 (git submodule) through a narrow C shim. It uses deterministic greedy decoding, re-verifies the model's SHA-256, rejects symlinked models, and exits when stdin closes.
- **Worker wire protocol v1:** 4-byte length-prefixed JSON envelopes plus raw f32 PCM, all bounded. Includes golden framing tests and a parent-side `WorkerClient` with hard-kill cancellation, timeouts and job/epoch fencing.
- **Audio pipeline:**
  - CPAL capture into a bounded lock-free ring; faults abort the session.
  - Mono downmix and stateful FFT resampling to 16 kHz.
  - earshot VAD with an energy floor.
  - Deterministic endpointer with pre-roll, silence finish, trailing pad, no-speech timeout, minimum speech length and a 120 s cap.
- **macOS platform adapter (unverified on hardware):**
  - Frontmost app, window and focused-element identity via NSWorkspace and Accessibility.
  - Secure-field detection.
  - Physical modifier-release wait.
  - Unicode clipboard write and a single Cmd+V.
  - Other platforms report an unknown target, so they never auto-paste.
- **Tauri shell (unverified on hardware):**
  - Tray with explicit Quit; single-instance.
  - Toggle and cancel global shortcuts with key-repeat filtering.
  - Non-focusable HUD with a before/after target-identity probe.
  - Exclusive microphone test with a level meter.
  - In-app worker self-test.
  - Per-window command capabilities; the HUD gets no commands.
- **M0 manual probes:** `mic_probe`, `vad_runs`, `m0_target_probe`, and `m0_paste_probe` (behind the `m0-diagnostics` feature).
- **Versioning:** `scripts/version.mjs` with `pnpm version:check` and `pnpm version:bump`, plus this changelog.
- **Docs:**
  - `docs/IMPLEMENTATION_STATUS.md`.
  - ADR-010: earshot VAD on 16 ms frames, replacing the unmaintained WebRTC VAD candidate.
  - ADR-011: capture always binds a concrete device id.
  - `CLAUDE.md`.

### Known limitations

- The Tauri shell and the macOS adapter have been compiler-checked but never run. HUD focus behavior, target identity, paste delivery, microphone permission and signing are all unproven.
- No dictation session flow yet: there is no coordinator, config persistence, model manager or onboarding (milestone M1 and later).
- Performance numbers so far come from a Linux x86_64 development host, not the reference hardware.

[Unreleased]: https://github.com/cahya-wirawan/dictation-app/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/cahya-wirawan/dictation-app/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/cahya-wirawan/dictation-app/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/cahya-wirawan/dictation-app/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/cahya-wirawan/dictation-app/releases/tag/v0.1.0
