# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Current state

Native local desktop dictation utility (tray/menu-bar app): global shortcut → native mic capture → silence endpointing → local whisper.cpp transcription → one safe paste into the original focused app, or an in-memory draft fallback.

Stack (fixed by the spec, do not substitute): Rust + Tauri 2 + Vite + React + strict TypeScript. No Python runtime, no cloud inference, no localhost HTTP service, no browser mic APIs. Platform priority: macOS Apple Silicon → Windows x64 → Linux X11 (experimental).

Milestone M0 is **partially complete**. `docs/IMPLEMENTATION_STATUS.md` is the source of truth for exact pins, what was verified on which machine, measurements and the open macOS questions. Update it with every milestone. macOS code (`crates/platform/src/macos.rs`, the HUD, the Tauri shell) has only been compiler-checked, never run.

## Commands

```sh
git submodule update --init --recursive   # vendor/whisper.cpp (pinned v1.9.5); required to build the worker
pnpm install

cargo test                                # default members only (excludes src-tauri, which needs webkit2gtk on Linux)
cargo test -p dictation-audio --test endpoint_fixtures pause_shorter   # single test
cargo test -p transcription-worker -- --nocapture                      # prints inference timings
cargo clippy --workspace --all-targets --all-features
cargo fmt --all
cargo clippy -p dictation-platform --target aarch64-apple-darwin --all-targets --all-features   # type-check macOS adapter from Linux

pnpm typecheck && pnpm lint && pnpm build # frontend (strict TS, typescript-eslint strictTypeChecked)
pnpm test                                 # vitest + Testing Library (jsdom); mock ../ipc in component tests

PROFILE=debug ./scripts/stage-worker.sh   # must run before building src-tauri (Tauri externalBin sidecar)
DICTATION_MODEL_PATH=$PWD/.models/ggml-base.bin pnpm tauri dev
```

- Model-dependent worker tests read `.models/ggml-base.bin` (gitignored; `./scripts/fetch-model.sh` downloads and verifies it) and **skip with a message** if it's missing. Override with `DICTATION_TEST_MODEL` / `DICTATION_TEST_MODEL_SHA256`.
- Release and CI worker builds set `DICTATION_GGML_NATIVE=OFF`; the default `ON` tunes ggml to the build machine's CPU. `OFF` means x86-64-v3 on x86_64 and Accelerate on macOS. `.cargo/config.toml` pins `MACOSX_DEPLOYMENT_TARGET=13.0`; keep it equal to `minimumSystemVersion` in tauri.conf.json.
- CI (`.github/workflows/ci.yml`) pins actions by commit SHA (docs/10 §2), so update the SHA and the tag comment together. A macOS test machine is reachable as `ssh 192.168.10.203`. Its SSH sessions have no GUI or TCC permissions; tools live in `~/.dictation-tools` (add `node/bin` and `cmake/CMake.app/Contents/bin` to PATH); the working copy is `~/dictation-app-ci`, synced with rsync.
- Building src-tauri on a Linux machine without webkit2gtk/gtk3/libsoup3 fails at pkg-config. For a type-check only, point `PKG_CONFIG_PATH` at stub `.pc` files (see IMPLEMENTATION_STATUS.md); never link or ship that way.
- Manual M0 probes on macOS: `examples/mic_probe.rs` (audio), `examples/m0_target_probe.rs` and `examples/m0_paste_probe.rs` (platform; the paste probe requires `--features m0-diagnostics` and must never become an IPC command).
- Versioning: SemVer, with a single source in `[workspace.package].version` (crates use `version.workspace = true`, Tauri inherits it, and `package.json` mirrors it). Never edit versions by hand. Add entries under `## [Unreleased]` in `CHANGELOG.md` as you make user-visible changes; `pnpm version:bump <major|minor|patch|X.Y.Z>` cuts a release, and `pnpm version:check` verifies everything agrees. Pre-1.0 policy and the separately versioned contracts (worker `PROTOCOL_VERSION`, config `schema_version`, event `schemaVersion`) are described at the top of CHANGELOG.md.
- Benchmarks and soak (docs/08): `cargo build --release -p dictation-bench -p transcription-worker`, then `target/release/dictation-bench corpus|nospeech|soak --worker <abs worker> --model <model> --sha <sha256> …`. Usage is in the header of `crates/bench/src/main.rs`; `crates/bench/corpus/jfk.tsv` is only a smoke corpus. The coordinator soak test is `cargo test -p dictation-coordinator --test soak`, about 45 s in debug.
- Release (docs/RELEASE.md): `pnpm notices` regenerates `THIRD_PARTY_NOTICES.md`, which is committed and bundled. `pnpm notices:check`, run in CI, fails if it is stale or if a shipped dependency has a non-permissive license. Run it after any dependency change. `scripts/verify-macos-bundle.sh <app> [--distribution]` checks signature, hardened runtime and entitlements. Tags trigger `.github/workflows/release.yml`, which creates a draft release. A DMG builds headless over SSH with `CI=true pnpm tauri build --bundles app,dmg`.
- Integration test files start with `#![allow(clippy::unwrap_used, ...)]`; the workspace warns on unwrap/expect/panic in non-test code.

## Spec documents

- `docs/01`–`docs/12` are the authoritative individual documents; `README.md` gives the reading order and a summary.
- `DICTATION_MASTER_SPEC.md` is a single-file concatenation of README + all docs (each section marked `<!-- Source: ... -->`). If you edit a doc, keep the master spec in sync.
- `docs/11_AGENT_HANDOFF.md` is written for coding agents: it defines the first task (milestone M0 spike) and the non-negotiable invariants. Read it before starting implementation work.
- Authority on conflicts: PRD (01) defines behavior; State/Contracts (04) defines ordering and invariants; Platform/Delivery (05) defines delivery safeguards. If they conflict, stop the affected work and write a small ADR (in `docs/09_ADRS_AND_RISKS.md` style) — never silently weaken a safeguard.
- Numbers in the spec are proposals/targets, not measured results. Schemas and type examples are design contracts, not verified API calls — check real crate/whisper.cpp headers instead of guessing method names from the prose.

## Planned architecture (docs/03, docs/04)

- **Single Rust session coordinator (actor)** is the only component allowed to advance a session or authorize delivery. Audio, inference, shortcut and UI callbacks only send typed messages to it. The UI renders coordinator snapshots and can never decide to paste.
- **State model**: `Readiness` (mic/model/worker/shortcut/delivery capabilities) is kept separate from `SessionPhase` (Idle, Starting, Listening, Finalizing, Transcribing, Delivering, DraftReady, Recovering). Every async response is fenced on session ID + generation + worker epoch + allowed phase; late results are discarded.
- **Audio pipeline**: CPAL callback copies into a preallocated bounded ring (real-time code: no locks with unbounded wait, I/O, or event emission) → DSP worker downmixes, resamples statefully to 16 kHz mono, 20 ms frames, VAD + energy floor, 300 ms pre-roll, 1000 ms silence endpoint, 120 s cap. The mic is open only during active capture.
- **Inference**: a separate bundled `dictation-worker` process links a pinned whisper.cpp (`vendor/whisper.cpp/`). It talks over inherited stdin/stdout pipes using 4-byte LE length + UTF-8 JSON envelopes (max 256 KiB), with raw float32 PCM following `Transcribe`. Cancel = kill the worker and restart it, not a cooperative abort. Wire types live in `crates/transcription-protocol` and are golden-tested.
- **Delivery**: a target token (process/window/focused element) is captured at shortcut press and re-validated after inference and just before dispatch. Clipboard write → recheck → mark committed in ledger (`not_attempted|committed|sent|uncertain`) → one paste chord. No automatic retry after commit, no auto-Enter, no forced refocus; terminals are excluded by default.
- **IPC**: Rust serde types are the contract source; TypeScript bindings are generated and checked in. The command list is in docs/04 §3. Events are scoped per window, carry sequence numbers, and `get_snapshot` is authoritative.
- **Layout**: `src/` (React UI; `ipc.ts` is the hand-written contract and should be generated from Rust in M1), `hud.html` + `src/hud/` (HUD window), and `src-tauri/` (shell: tray, shortcuts, IPC and notifier over the coordinator). The crates are:
  - `crates/domain`: ids, phases, chord repeat filter.
  - `crates/transcription-protocol`: wire types, framing, parent-side `WorkerClient`.
  - `crates/transcription-worker`: the `dictation-worker` binary; a C shim (`shim.c`) keeps whisper.cpp's param structs out of Rust FFI.
  - `crates/audio`: capture, resample, VAD, endpointer, pipeline.
  - `crates/platform`: target probe, clipboard and paste. There is a macOS implementation plus an `unsupported.rs` fallback that always yields Unknown, which means no auto-paste.
  - `crates/models`: the approved model manifest (the only source of model URLs and hashes), the verified resumable downloader behind a `Fetcher` trait, and the private model store. The app's commands are in `src-tauri/src/models.rs`.
  - `crates/coordinator`: the actor (`actor.rs`), production adapters (`adapters.rs`) and the settings file store. It lives here rather than in `src-tauri` so it can be tested without a webview.
- **Session core**: `dictation_domain::session::Machine` is a pure reducer: `step(Input) -> Vec<Effect>`, with no clocks or I/O. The actor in `crates/coordinator/src/actor.rs` runs it on one thread and turns effects into adapter calls (`CaptureAdapter`, `TranscriberAdapter`, `DeliveryAdapter`, `Notifier`). Adapters report back only by sending `AdapterEvent`s. Follow-up inputs are queued, never stepped re-entrantly. New behavior belongs in the reducer first (with fixtures in `crates/domain/tests/session_machine.rs`), then in an adapter. Delivery is `PlatformDelivery<P: TargetPlatform>` (`crates/coordinator/src/delivery.rs`). Its thread owns `!Send` native target tokens, and the tests drive it with a scripted `FakePlatform` (`crates/coordinator/tests/delivery.rs`). Only a `DeliveryPrepared(Ok)` processed by the reducer commits the ledger and emits the single `DispatchPaste`.
- **Windows**: `main` (settings and onboarding, shown on first run until `ui.onboarding_complete`), `review` (draft only), and `hud` (events only). Each has its own capability file. Draft commands also check the window label in Rust (`draft_window`).
- **Capability scoping**: `src-tauri/build.rs` declares every app command in an `AppManifest`. `src-tauri/capabilities/*.json` grants them per window, and the HUD gets none. A new command must be added in both places, or it is uncallable.
- **Diagnostics**: `crates/coordinator/src/diagnostics.rs` records only outcome codes, job durations and the worker's Hello facts. It does this through `RecordingNotifier` and `WorkerTranscriber::with_diagnostics`. Never feed it text, paths, device names or target identities. The soak test asserts that a dictated token is absent from the report.
- **Session-state hook**: `dictation_platform::watch_suspend` turns sleep, lock, user switch and logout into `Command::Suspend`. On macOS it uses NSWorkspace and distributed notifications, which are unverified on hardware. Other platforms report nothing yet.
- **Deviations already recorded as ADRs** in docs/09: ADR-010 (earshot VAD on 16 ms / 256-sample frames instead of WebRTC VAD on 20 ms) and ADR-011 (capture always binds a concrete device id, because CPAL silently reroutes default-device streams).

## Non-negotiable invariants (from docs/11)

- No idle mic capture. No transcript/audio in logs, persisted history, analytics or network requests.
- The frontend gets no raw key injection, process execution, unrestricted file read, arbitrary URL download, PCM input or target PIDs.
- No auto-paste to a changed or unknown target. Cancel invalidates delivery eligibility before resource teardown.
- No silent mic substitution after disconnect, no cloud fallback on worker failure, and no re-running canceled audio.
- Do not claim paste success from an API return value. Verify the text in the receiving app. Mocked transcription or a compiled window does not count as completion.
- Rust: typed errors, no unwrap/panic on user/device/model inputs, narrow RAII wrappers around OS/FFI handles. Domain tests must not require a webview.
