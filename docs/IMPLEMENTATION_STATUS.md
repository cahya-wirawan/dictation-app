# Implementation status

Last updated: 12 November 2026 · **M0 partially complete · M1 (domain and capture) implemented, native GUI run pending** · Version 0.2.0

**M0 is not done.** Its exit evidence must come from a macOS Apple Silicon release-like build (docs/07 §2), and no Mac has run this code yet. This round ran on a headless Linux host. Everything that can be proven without a display, microphone or macOS is proven below. Everything else is written but marked **unverified**.

## Where this ran

### macOS build host (via SSH, no GUI session)

| Item | Value |
|---|---|
| Machine | Apple M2 Pro (10 cores), 32 GB RAM |
| OS | macOS 26.6.2 (25G83), Xcode command line tools at `/Applications/Xcode.app` |
| Toolchains | Rust 1.93.0; Node 24.22.3, cmake 4.4.5 and pnpm (corepack) installed in user space under `~/.dictation-tools` (remove that directory to undo) |
| Limitation | SSH sessions cannot be granted Microphone or Accessibility (TCC) permission and cannot see the GUI. Probes that need either still require someone at the Mac. |

On this Mac, these pass:

- `cargo clippy --workspace --all-targets --all-features -D warnings`
- `cargo test --workspace`: all 46 tests, with real CPU and Metal inference
- `pnpm tauri build --bundles app`

The Tauri crate compiles and links on macOS, and the bundle layout is correct.

### Linux development host

| Item | Value |
|---|---|
| Host | Ubuntu 22.04.4 LTS, kernel 6.8.0-138, x86_64, Intel Core i3-10100 (4C/8T, 3.6 GHz), 62 GB RAM |
| Session | tty only: no X11/Wayland display. ALSA reports no accessible sound cards. No sudo. |
| Missing system libs | webkit2gtk-4.1, gtk+-3.0, libsoup-3.0 (Tauri cannot be built or run here) |
| Toolchains | Rust 1.93.0 (pinned in `rust-toolchain.toml`), Node 24.18.0, pnpm 10.0.0, cmake 3.22.1, gcc 11.4 / clang 14 |

## Pinned versions

| Component | Version | Notes |
|---|---|---|
| whisper.cpp | **v1.9.5**, commit `d1be6fde11ac6e0407606b4e42fe72d34add8037` | git submodule `vendor/whisper.cpp`; static libs, OpenMP off, BLAS off, Metal off unless `--features metal` on macOS |
| Model `whisper-base-multilingual` | `ggml-base.bin`, 147,951,465 bytes, SHA-256 `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` | from `huggingface.co/ggerganov/whisper.cpp`; stored in gitignored `.models/` for tests |
| tauri / tauri-build | 2.12.5 / 2.7.4 | `tray-icon` feature |
| tauri-plugin-global-shortcut | 2.4.1 | |
| tauri-plugin-single-instance | 2.5.5 | |
| cpal | 0.18.2 | see ADR-011 |
| rubato / audioadapter-buffers | 5.0.1 / 5.2.0 | FFT resampler, fixed input chunk ≈10 ms |
| earshot | 1.2.2 | VAD; replaces webrtc-vad (ADR-010) |
| rtrb | 0.4.1 | SPSC ring for the audio callback |
| objc2 / objc2-app-kit / -application-services / -core-graphics / -core-foundation / -foundation | 0.6.5 / 0.3.2 | macOS adapter |
| serde / serde_json / thiserror / sha2 / getrandom / hound / cmake / cc | 1.0.229 / 1.0.154 / 2.0.21 / 0.11.1 / 0.4.3 / 3.5.1 / 0.1.58 / 1.11.1 | |
| React / Vite / @vitejs/plugin-react | 19.3.1 / 8.3.8 / 6.3.0 | |
| TypeScript | **6.0.3** (not 7.x) | typescript-eslint 8.72.2 supports `<6.1.0` |
| @tauri-apps/api / cli | 2.12.4 / 2.12.4 | |
| eslint / typescript-eslint | 10.14.0 / 8.72.2 | `strictTypeChecked` |

Lockfiles: `Cargo.lock`, `pnpm-lock.yaml`.

## M1: domain and capture

The exit criterion is "deterministic state/audio fixtures and no idle capture" (docs/07 §2). It is met by automated tests on Linux and on the M2 Pro. What's still open is a person running the app on the Mac: microphone permission and a real spoken session.

| Work package | Where | Evidence |
|---|---|---|
| **A1:** pure session reducer | `crates/domain/src/session.rs` | 21 transition fixtures, plus a seeded phase-aware fuzz test (400 runs × 60 steps; about 1,000 dispatches and 140 pre-commit cancels per run of the suite) asserting its invariants. |
| **Config:** schema, validation, persistence | `crates/domain/src/config.rs`, `crates/coordinator/src/config_store.rs` | Ranges from docs/01 §5. RFC 7386 merge-patch with revision conflicts. 0600 file in a 0700 directory, fsynced temp sibling + rename. Corrupt or oversized files are backed up. Future-schema files are never overwritten. |
| **A2:** DSP worker | `crates/audio/src/dsp.rs` | 9 fault-injection tests. The microphone is closed before every terminal event; overflow, disconnect and NaN abort with no partial utterance; dropping a session closes the device; the meter stays ≤15 Hz. |
| A3: endpointing | done in M0 | |
| **Coordinator actor and adapters** | `crates/coordinator/src/{actor,adapters}.rs` | 10 end-to-end tests with a fake microphone. They assert `capture_open == false` and the fake mic closed after every path. One test uses the real worker on jfk.wav, including cancel during inference and respawn with a new epoch. |
| **Tauri wiring** | `src-tauri/src/lib.rs`, `src/` | Covered below. |

**Tauri wiring:**

- **Inputs:** the toggle and cancel shortcuts and the tray Cancel item send commands to the coordinator.
- **HUD:** shown and hidden by phase; it gets phase and level events only.
- **Settings window:** shows phase and outcomes in the docs/02 §5 copy, holds the memory-only draft (edit, copy, discard) and the live settings form (device, language, silence, no-speech timeout, HUD).
- **Scoping:** every new command is permitted for the main window only.
- **Verification:** strict clippy passes on Linux (stub libs) and on macOS. The app has not been launched in a GUI session.

**Deliberate M1 limits:**

- **Delivery is copy-only.** `CopyOnlyDelivery` never pastes. Automatic delivery and the explicit paste-draft shortcut arrive in M3 (C1–C3); the reducer and actor already implement the commit ledger for them.
- **No sleep/lock/logout hook yet.** `Input::Suspend` exists and is tested; the OS notification adapter still has to call it.
- **IPC request ids are not deduplicated yet** (docs/04 §3), and TypeScript bindings are still hand-written in `src/ipc.ts`.
- **The coordinator lives in `crates/coordinator`**, not `src-tauri/src/coordinator/` as docs/03 §2 suggests, so it can be tested without a webview (docs/07 §5).
- **The worker respawns in the background after every hard cancel.** On CPU that takes about 165 ms. With Metal it would take about 5 s, so Metal stays off.

**Bug found by the tests:** effects that fed follow-up inputs straight back into the reducer reported `Listening` before `Starting` to the UI. The actor now queues follow-up inputs until all of the current input's effects have run.

## M0 tasks

Status key: ✅ verified on this host · 🟡 written and compiler-checked, not run · ⛔ not done.

| Task | Status | Evidence / limitation |
|---|---|---|
| T0.1 Scaffold, strict lint/typecheck, lockfiles | ✅ | Cargo workspace, Vite + React + strict TS. All lint and type checks are clean. The Tauri crate compiles, links and bundles on macOS (M2 Pro). It has not been launched in a GUI session. CI: `.github/workflows/ci.yml`. |
| T0.2 Capture 48/44.1 kHz in Rust; permission in signed bundle | 🟡 | `crates/audio/src/capture.rs` + `examples/mic_probe.rs`. No microphone on this host. Signed-bundle permission test not done. The format → mono → 16 kHz path is verified with fixtures (below). |
| T0.3 Toggle/cancel chords, repeat filter, modifier release | ✅ logic / 🟡 native | `ChordFilter` unit-tested (holding the chord starts once). Registration code in `src-tauri/src/lib.rs` is unrun. The macOS modifier-release poll (`CGEventSourceFlagsState`, HID state) compiles for aarch64-apple-darwin. |
| T0.4 Non-activating HUD | 🟡 | HUD window: `focusable: false`, `focus: false`, always-on-top, `ActivationPolicy::Accessory`. Whether `WebviewWindow::show()` on macOS leaves the frontmost app, window, field and caret unchanged is **the open question**; the toggle-shortcut probe measures it. |
| T0.5 Editable element identity (TextEdit, browser textarea) | 🟡 | `crates/platform/src/macos.rs`: NSRunningApplication (`isEqual:`) + pid + launch date; AX focused window and focused element compared with `CFEqual`; `AXSecureTextField` → Secure. Compiles and passes clippy for aarch64-apple-darwin. Not run. |
| T0.6 Unicode clipboard + one paste, inspect receiver | 🟡 | `examples/m0_paste_probe.rs`, behind the `m0-diagnostics` feature. Fixed text only. Reads the destination back via AX `AXValue` and reports `RECEIVED` / `DUPLICATED`. No IPC path exists to it. |
| T0.7 whisper.cpp CPU worker, Base model, 10 s fixture, kill/restart | ✅ (Linux x86_64 and Apple M2 Pro) | See measurements. Bundled sidecar naming verified in the `.app`; signing not done. |
| T0.8 VAD maintenance/license/frames; resampler behavior | ✅ | webrtc-vad rejected (unmaintained since 2019) → earshot (ADR-010). Resampler: 16/22.05/44.1/48/96 kHz, random and 1-sample blocks, single flush, exact output length, 440 Hz preserved. |
| Signing experiment | ⛔ | Needs a Mac and a Developer ID. |
| Handoff item 7: this file | ✅ | |

## Measurements: Apple M2 Pro, macOS 26.6.2

Portable worker build (`DICTATION_GGML_NATIVE=OFF`; ggml with Accelerate), JFK fixture 11.0 s, run in isolation (`--test-threads=1`), two runs:

| Backend | Model load (incl. worker SHA-256 re-hash) | Warm inference, 11 s fixture | RTF |
|---|---|---|---|
| CPU | 163–166 ms | 385–656 ms (median ≈ 410 ms) | ≈ 0.04 |
| Metal (`--features metal`) | **5,259 ms** | first job 616 ms, then 108–111 ms | ≈ 0.01 |

- When model tests run in parallel, CPU contention inflates the warm numbers to 0.4–5.6 s. Measure in isolation.
- Metal's cold load is about 5 s, and every hard cancel kills and respawns the worker. With Metal on, a cancel would add about 5 s before the next dictation can start, unless the next worker is pre-spawned or Metal's shader-compilation cache can be reused. Investigate before Metal becomes a default; CPU already meets the 10 s stop-to-dispatch target with a wide margin.
- Release bundle: `Local Dictation.app` is 11.7 MiB, version 0.1.0, with the arm64 `dictation-worker` sidecar in `Contents/MacOS`. It is ad-hoc linker-signed with identifier `dictation_app-<hash>`, not the bundle id; real signing must set the identity and entitlements.
- Not yet measured: peak RSS of parent plus worker, trigger latency, idle CPU (docs/08 §5).

## Measurements (Linux x86_64 host, *not* reference hardware)

Worker built with `GGML_NATIVE=ON` (tuned to this CPU). whisper.cpp itself is compiled `Release` even inside Rust debug builds. Fixture: `vendor/whisper.cpp/samples/jfk.wav`, 11.0 s, 16 kHz mono, public domain.

| Metric | Result |
|---|---|
| Model load (worker re-hash + `whisper_init`) | 729 ms |
| Warm inference, 11 s fixture, 3 runs, `en` | 977 / 882 / 929 ms (RTF ≈ 0.08–0.09) |
| Output | "And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country." Identical across runs; auto-detected language `en`. |
| Hard cancel during a 30 s job | Returns `WorkerExited` within 5 s; process reaped; a fresh worker (new epoch) transcribes correctly |
| Timeout (200 ms on a 60 s job) | Worker killed and reaped |
| Worker binary size (debug) | 12.8 MB |

These numbers are **not** evidence for the PRD targets (trigger p95, stop-to-dispatch p95, RSS, idle CPU). Those require the reference Mac (docs/08 §5).

## Verified behavior (46 tests, `cargo test`)

- **Protocol (B1):** golden frame bytes; oversized or zero length rejected before allocation; truncated header/body; unknown fields and variants rejected; NaN or short PCM rejected; `pcm_bytes == 4 × sample_count`; a worst-case 32 KiB result fits the 256 KiB envelope.
- **Worker (B2/B3):** Hello reports the pinned revision; exits on stdin EOF; a malformed length gets `ProtocolViolation` and exit 2 with bounded stderr; a request without a loaded model gets a typed error and the stream stays framed; wrong epoch, NaN and empty PCM are refused client-side; a tampered hash and a symlinked model are never loaded; greedy decoding is deterministic.
- **Endpointing (A3):** a 900 ms pause stays one phrase; finish exactly at 1000 ms of silence with a 200 ms trailing pad; 8 s no-speech timeout; isolated impulses never start a session; pre-roll is present; a burst under 200 ms is TooShort; caps at 2 s and 120 s finalize once, within the worker's `MAX_SAMPLES`; explicit stop uses the available audio and is idempotent; stop before speech gives NoSpeech.
- **Pipeline:** 48 kHz stereo JFK with random block sizes is endpointed end to end through downmix → resample → VAD → endpointer.
- **Unsupported platform:** target identity is Unknown → auto-delivery never allowed; modifier wait returns false; paste is unsupported.

## Findings that change or sharpen the spec

1. **The 1000 ms default splits deliberately paced speech.** jfk.wav has two natural pauses of 1040 ms and 1024 ms (measured with `examples/vad_runs.rs`). At the default the phrase ends after "And so my fellow Americans" (2.38 s); at 1500 ms the whole clip is one 11.1 s utterance. This is correct per spec, but it is an early signal for silence-timeout calibration on the corpus (docs/08 §3).
2. **CPAL default-device streams follow OS rerouting** → ADR-011 (always bind a concrete device id).
3. **The earshot frame size is 16 ms**, not 20 ms → ADR-010.
4. **TypeScript 7 is not usable with typescript-eslint yet** → pinned 6.0.3.
5. **`NSRunningApplication.processIdentifier` is documented by Apple as unsuitable for comparing processes** → the target token also keeps the `NSRunningApplication` and compares with `isEqual:`.
6. **macOS link requirements:** ggml's CPU backend uses vDSP from Accelerate even with BLAS off, so the worker links `-framework Accelerate`. whisper.cpp objects were also being built for the SDK's macOS 27.2 while rustc linked for 11.0. `.cargo/config.toml` now sets `MACOSX_DEPLOYMENT_TARGET=13.0` for rustc, cmake and cc, matching the bundle's `minimumSystemVersion`.
7. **The portable x86 build needs an explicit SIMD baseline.** With `GGML_NATIVE=OFF`, ggml falls back to SSE only: 10–53 s on the JFK fixture versus 0.9 s native. The portable x86_64 build now enables x86-64-v3 (AVX, AVX2, FMA, F16C, BMI2; Haswell 2013+), which brings it back to 1.0–1.3 s. This becomes the Windows x64 minimum CPU unless an ADR chooses runtime CPU dispatch (`GGML_BACKEND_DL` + `GGML_CPU_ALL_VARIANTS`) instead.
8. **The worker rejects symlinked model paths** (`symlink_metadata`). A residual window remains between hashing and `whisper_init_from_file` re-opening the same path. That's acceptable only because the model directory is app-private; M2 should load from the already-open, verified file handle or a private copy.

## Unresolved feasibility questions (need a Mac)

- Does the Tauri HUD (`focusable: false`, `show()`) leave the frontmost app, key window, focused element and caret unchanged? If not: tray-only status (docs/07 §6), or an NSPanel adapter via an ADR.
- Does `AXFocusedUIElement` identity stay stable and `CFEqual`-comparable in TextEdit, and in Safari/Chrome textareas? Does a same-window field change get detected?
- Does `CGEventPost` of Cmd+V (flag on the V events, no synthetic modifier keys) paste exactly once in TextEdit, and does AX `AXValue` read the result back?
- Microphone TCC prompt and denial behavior inside a signed `.app`; whether the bundled `dictation-worker` sidecar keeps working under signing/notarization (docs/10).
- RSS for Base on Apple Silicon. CPU latency is measured above. Metal is fast when warm, but its about 5 s cold load per worker spawn is a blocker for the cancel-then-restart flow.

## Manual checklist on the Mac (macOS 13+, Apple Silicon)

```sh
git submodule update --init --recursive
pnpm install
./scripts/fetch-model.sh                     # downloads + verifies size and SHA-256

cargo test                                   # all 46 tests, incl. real inference
cargo run -p dictation-audio --example mic_probe -- --seconds 8         # T0.2
cargo run -p dictation-platform --example m0_target_probe               # T0.5
cargo run -p dictation-platform --features m0-diagnostics --example m0_paste_probe   # T0.6

PROFILE=debug ./scripts/stage-worker.sh
DICTATION_MODEL_PATH=$PWD/.models/ggml-base.bin pnpm tauri dev          # T0.3/T0.4/T0.7 in-app
DICTATION_GGML_NATIVE=OFF ./scripts/stage-worker.sh && pnpm tauri build # release-like bundle
```

Record each result here (exact macOS build, Mac model, app versions):

- [ ] mic_probe: native rate/channels/format; the level moves; utterance endpointed; permission prompt and denial path
- [ ] Toggle chord held down → one probe only; cancel chord hides the HUD
- [ ] HUD probe in TextEdit, Safari textarea, Chrome textarea: result `Same`; caret position unchanged (visual check)
- [ ] Target probe: app switch → `AppChanged`; new window → `WindowChanged`; other field in the same window → `FieldChanged`; password field → `Secure`
- [ ] Paste probe in TextEdit: `RECEIVED=true DUPLICATED=false`, Unicode exact (umlauts, em dash, emoji, CJK)
- [x] Release-like bundle builds on macOS 26.6.2 / M2 Pro; sidecar present (verified over SSH)
- [ ] Settings → "Transcribe test fixture" in the release-like bundle: engine v1.9.5, fixture text correct, worker reaped
- [ ] Quit from the tray: no `dictation-worker` process left (`pgrep dictation-worker`)
- [ ] **M1 session:** with the model installed in the app data `models/` directory, press the toggle in TextEdit and say a sentence. Check: the HUD shows Listening and the meter moves; it finishes after a pause; "Transcribing locally…" appears; the settings window shows the text; Copy puts it on the clipboard and the draft clears.
- [ ] **M1 cancel:** cancel during Listening and during Transcribing (tray or chord). The microphone indicator in the menu bar turns off, and the next session works.
- [ ] **M1 no idle capture:** with the app idle for a minute, the macOS microphone indicator stays off.
- [ ] Accessibility denied → target capture returns `NotTrusted` and nothing is pasted

## Not in M0 (next: M1)

Coordinator actor and session state machine, generated TS bindings, config persistence, the real start/stop dictation flow, the model manager (download/import), onboarding, and Windows/Linux adapters. The worker self-test embeds the JFK fixture via `include_bytes!`; remove it or compile-gate it before the product build.
