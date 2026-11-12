# Implementation status

Last updated: 12 November 2026 · Milestone: **M0 (feasibility and pins) — partially complete**

**M0 is not done.** Its exit evidence must come from a macOS Apple Silicon release-like build (docs/07 §2), and no Mac has run this code yet. This round ran on a headless Linux host. Everything that can be proven without a display, microphone or macOS is proven below. Everything else is written but marked **unverified**.

## Where this round ran

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

## M0 tasks

Status key: ✅ verified on this host · 🟡 written and compiler-checked, not run · ⛔ not done.

| Task | Status | Evidence / limitation |
|---|---|---|
| T0.1 Scaffold, strict lint/typecheck, lockfiles | ✅ / 🟡 | Cargo workspace, Vite + React + strict TS. `pnpm typecheck`, `pnpm lint`, `pnpm build`, `cargo clippy --workspace --all-targets --all-features` all clean. The Tauri crate was only type-checked, for Linux, using pkg-config stubs (no webkit); it was never linked or run. tauri-build did validate both capability files. |
| T0.2 Capture 48/44.1 kHz in Rust; permission in signed bundle | 🟡 | `crates/audio/src/capture.rs` + `examples/mic_probe.rs`. No microphone on this host. Signed-bundle permission test not done. The format → mono → 16 kHz path is verified with fixtures (below). |
| T0.3 Toggle/cancel chords, repeat filter, modifier release | ✅ logic / 🟡 native | `ChordFilter` unit-tested (holding the chord starts once). Registration code in `src-tauri/src/lib.rs` is unrun. The macOS modifier-release poll (`CGEventSourceFlagsState`, HID state) compiles for aarch64-apple-darwin. |
| T0.4 Non-activating HUD | 🟡 | HUD window: `focusable: false`, `focus: false`, always-on-top, `ActivationPolicy::Accessory`. Whether `WebviewWindow::show()` on macOS leaves the frontmost app, window, field and caret unchanged is **the open question**; the toggle-shortcut probe measures it. |
| T0.5 Editable element identity (TextEdit, browser textarea) | 🟡 | `crates/platform/src/macos.rs`: NSRunningApplication (`isEqual:`) + pid + launch date; AX focused window and focused element compared with `CFEqual`; `AXSecureTextField` → Secure. Compiles and passes clippy for aarch64-apple-darwin. Not run. |
| T0.6 Unicode clipboard + one paste, inspect receiver | 🟡 | `examples/m0_paste_probe.rs`, behind the `m0-diagnostics` feature. Fixed text only. Reads the destination back via AX `AXValue` and reports `RECEIVED` / `DUPLICATED`. No IPC path exists to it. |
| T0.7 whisper.cpp CPU worker, Base model, 10 s fixture, kill/restart | ✅ (Linux x86_64) | See measurements. Not yet measured on Apple Silicon; bundled-executable naming/signing not verified. |
| T0.8 VAD maintenance/license/frames; resampler behavior | ✅ | webrtc-vad rejected (unmaintained since 2019) → earshot (ADR-010). Resampler: 16/22.05/44.1/48/96 kHz, random and 1-sample blocks, single flush, exact output length, 440 Hz preserved. |
| Signing experiment | ⛔ | Needs a Mac and a Developer ID. |
| Handoff item 7: this file | ✅ | |

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
6. **The worker rejects symlinked model paths** (`symlink_metadata`). A residual window remains between hashing and `whisper_init_from_file` re-opening the same path. That's acceptable only because the model directory is app-private; M2 should load from the already-open, verified file handle or a private copy.

## Unresolved feasibility questions (need a Mac)

- Does the Tauri HUD (`focusable: false`, `show()`) leave the frontmost app, key window, focused element and caret unchanged? If not: tray-only status (docs/07 §6), or an NSPanel adapter via an ADR.
- Does `AXFocusedUIElement` identity stay stable and `CFEqual`-comparable in TextEdit, and in Safari/Chrome textareas? Does a same-window field change get detected?
- Does `CGEventPost` of Cmd+V (flag on the V events, no synthetic modifier keys) paste exactly once in TextEdit, and does AX `AXValue` read the result back?
- Microphone TCC prompt and denial behavior inside a signed `.app`; whether the bundled `dictation-worker` sidecar keeps working under signing/notarization (docs/10).
- Apple Silicon CPU inference latency and RSS for Base; Metal as a gated optimization.

## Manual checklist on the Mac (macOS 13+, Apple Silicon)

```sh
git submodule update --init --recursive
pnpm install
mkdir -p .models && curl -L -o .models/ggml-base.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
shasum -a 256 .models/ggml-base.bin   # must equal the pinned SHA-256 above

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
- [ ] Settings → "Transcribe test fixture" in the release-like bundle: engine v1.9.5, fixture text correct, worker reaped
- [ ] Quit from the tray: no `dictation-worker` process left (`pgrep dictation-worker`)
- [ ] Accessibility denied → target capture returns `NotTrusted` and nothing is pasted

## Not in M0 (next: M1)

Coordinator actor and session state machine, generated TS bindings, config persistence, the real start/stop dictation flow, the model manager (download/import), onboarding, and Windows/Linux adapters. The worker self-test embeds the JFK fixture via `include_bytes!`; remove it or compile-gate it before the product build.
