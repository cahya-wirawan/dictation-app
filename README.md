# Local Dictation

Local Dictation is a small desktop dictation app that runs entirely on your computer. Press a shortcut, speak, and pause. The app transcribes what you said with a local [Whisper](https://github.com/openai/whisper) model ([whisper.cpp](https://github.com/ggml-org/whisper.cpp)) and pastes the text into the app and text field you started in. No account, no cloud, no recordings kept.

> **Status: pre-1.0 (v0.9.0).** The core features are implemented and tested in CI on macOS, Windows and Linux. The first hands-on report is in: on an Intel MacBook Pro (2019), dictation pasted automatically into Notes about 1–2 s after the user stopped speaking. Formal qualification on real desktops is still pending, and the macOS build is not notarized. Details, measurements and open items are in [docs/IMPLEMENTATION_STATUS.md](docs/IMPLEMENTATION_STATUS.md).

## Features

- **Global shortcut dictation.** Toggle to start, then stop by pausing (1 s of silence by default) or by pressing the shortcut again. A separate shortcut cancels.
- **Optional continuous dictation.** Keep listening after each phrase and paste every phrase as you pause, until you press the shortcut again or stop talking.
- **Pastes into the original field, exactly once.** The app records which app, window and text field were focused when you started. It pastes only if that same field is still focused when the text is ready. Otherwise it keeps a **draft** you can review, edit and copy.
- **Safe by default.** Never pastes into password fields, terminals or apps you exclude. Never presses Enter, never steals focus, never retries a paste.
- **Fully local.** Audio stays in memory and is never written to disk. Transcription runs on the CPU in a separate worker process. The only network access is a model download you start yourself.
- **Multilingual.** Whisper Base (148 MB, default) or Small (488 MB). Language detection is automatic, or you can fix English, German or Indonesian.
- **Lightweight.** A tray/menu-bar app with a small status HUD. The microphone is open only while you dictate.
- **Private.** No history, no analytics, no transcript logging. The support report contains only codes and timings.

## Platform support

| Platform | Status | Automatic paste |
|---|---|---|
| **macOS 13+**, Apple Silicon and Intel (universal) | Primary platform | Yes, with Accessibility permission |
| **Windows 10/11 x64** | Second platform | Yes, into standard Edit/Document fields of non-elevated apps |
| **Linux X11** | Experimental | No: text is kept as a draft and you copy it |
| **Linux Wayland** | Manual workflow | No; the global shortcut is unreliable, so start dictation from the app window |

The full matrix, and what each platform's support is based on, is in [docs/PLATFORM_SUPPORT.md](docs/PLATFORM_SUPPORT.md).

### Requirements

- **macOS:** 13 Ventura or later. Apple Silicon, or an Intel Mac with AVX2 (every Intel Mac that runs macOS 13 has it).
- **Windows:** 10 or 11, x64, with a CPU supporting AVX2/FMA (Intel Haswell or AMD Excavator/Zen and newer). WebView2 is included in Windows 11 and current Windows 10 builds; otherwise the installer downloads it once.
- **Linux:** Ubuntu 24.04 or similar, with WebKitGTK 4.1, GTK 3 and an AppIndicator library. X11, or Wayland with XWayland, for Copy.
- **All platforms:** a microphone, about 150 MB of disk space for the Base model (500 MB for Small), and an internet connection once to download a model, unless you import one.

## Installation

### macOS

1. Download `Local.Dictation_<version>_universal.dmg` from the [latest release](https://github.com/cahya-wirawan/dictation-app/releases/latest).
2. Open the DMG and drag **Local Dictation** to Applications.
3. The build is not notarized yet, so the first launch is blocked by Gatekeeper. Right-click the app and choose **Open**, or run:

   ```sh
   xattr -dr com.apple.quarantine "/Applications/Local Dictation.app"
   ```

Each release also includes `SHA256SUMS`, a CycloneDX SBOM and the third-party notices.

### Windows and Linux

Releases do not include Windows or Linux packages yet. You can build them from source (see below). The CI workflow also produces an unsigned per-user Windows installer (`*-setup.exe`) and an experimental `.deb` as build artifacts.

## Usage

### First run

The app lives in the menu bar (macOS) or the system tray (Windows, Linux). Setup opens automatically and walks you through these steps:

1. **Microphone:** allow access. On Windows, *Let desktop apps access your microphone* must be on.
2. **Model:** download Whisper Base, or import a `ggml-base.bin` or `ggml-small.bin` you already have. It is verified against a pinned checksum.
3. **Shortcut:** keep the default or choose your own.
4. **Automatic paste (macOS):** grant Accessibility in System Settings → Privacy & Security, or choose copy-only.
5. **Trial:** say a sentence to check the microphone and the model.

### Dictating

| Action | Default shortcut |
|---|---|
| Start / finish dictation | **Ctrl+Option+Space** (macOS) · **Ctrl+Alt+Space** (Windows, Linux) |
| Cancel | **Ctrl+Option+Escape** (macOS) · **Ctrl+Alt+Escape** (Windows, Linux) |

1. Click into a text field in any app.
2. Press the start shortcut and speak.
3. Pause, or press the shortcut again. The text is pasted into the field you started in.

With **continuous dictation** on (Settings → Recording), the app keeps listening after step 3 and pastes each further phrase when you pause. Press the shortcut again to stop.

If the app can't paste safely, the text becomes a **draft**. That happens when you switched app or field, the target is a password field or terminal, or the app couldn't identify the field. Open the draft from the tray menu with **Review text…** to edit or copy it. Drafts live only in memory and are cleared after 10 minutes, on quit, sleep, screen lock or logout.

### Settings

Open **Settings…** from the tray menu:

- **General:** status HUD, reduced motion, launch at login.
- **Recording:** microphone, silence timeout (0.5–2.5 s), no-speech timeout, continuous dictation, microphone test.
- **Shortcuts:** start/finish, cancel and an optional paste-draft shortcut.
- **Models:** language, model download, import, verify, unload.
- **Delivery:** automatic paste or copy-only, excluded apps.
- **Privacy:** what happens to your speech.
- **About:** versions, licenses, support report.

More help, troubleshooting and uninstall instructions are in [docs/SUPPORT.md](docs/SUPPORT.md).

## Privacy

- The microphone is open only while you dictate or run the microphone test.
- Audio and transcripts are never written to disk, logged, or sent anywhere.
- Transcription happens on your computer. The only network traffic is a model download you start yourself.
- **The clipboard is not private.** Pasting works through the system clipboard. Clipboard managers, sync and other apps can see text that passes through it.

## Building from source

### Prerequisites

- Rust 1.93 (pinned in `rust-toolchain.toml`), Node.js 24 with pnpm 10, CMake, and a C/C++ compiler: Xcode Command Line Tools, MSVC Build Tools or GCC/Clang.
- **Linux only:** `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev libasound2-dev`.

### Steps

```sh
git clone --recurse-submodules https://github.com/cahya-wirawan/dictation-app
cd dictation-app
pnpm install

# Development: build the transcription worker sidecar, then run the app.
PROFILE=debug ./scripts/stage-worker.sh
pnpm tauri dev

# Release build (portable CPU code).
export DICTATION_GGML_NATIVE=OFF
PROFILE=release ./scripts/stage-worker.sh && pnpm tauri build    # host platform
TARGET=universal-apple-darwin PROFILE=release ./scripts/stage-worker.sh \
  && pnpm tauri build --target universal-apple-darwin --bundles app,dmg   # macOS universal
```

### Tests and checks

```sh
./scripts/fetch-model.sh                  # optional: Whisper Base for the real-inference tests
cargo test                                # Rust (all crates except the Tauri shell)
cargo clippy --workspace --all-targets --all-features
pnpm typecheck && pnpm lint && pnpm test  # frontend
```

Benchmarks (WER/CER, latency, cold start, soak) use `dictation-bench`; see the header of `crates/bench/src/main.rs`. [CLAUDE.md](CLAUDE.md) has a fuller developer reference.

## Architecture

```
global shortcut ─► session coordinator (Rust actor) ─► audio capture ─► 16 kHz resample, VAD, endpointing
                         │                                                     │
                         │◄──────────── transcript ◄── dictation-worker (whisper.cpp, separate process)
                         ▼
           target re-check ─► clipboard ─► one paste chord     (or a draft in the review window)
```

- A single Rust coordinator owns the session state machine, a pure reducer, and is the only component that can authorize a paste. The React UI renders snapshots and cannot trigger a paste.
- Inference runs in a bundled worker process that talks over private pipes; there is no local HTTP server. Cancel kills and restarts the worker.

| Path | Contents |
|---|---|
| `src-tauri/` | Tauri shell: tray, shortcuts, IPC commands, windows |
| `src/` | React + TypeScript UI: onboarding, settings, review, HUD |
| `crates/domain` | Session state machine, configuration, shared types |
| `crates/coordinator` | Coordinator actor, adapters, delivery, diagnostics |
| `crates/audio` | Capture, resampling, voice activity detection, endpointing |
| `crates/platform` | Per-OS target identity, clipboard, paste, permissions, sleep/lock |
| `crates/transcription-protocol`, `crates/transcription-worker` | Worker wire protocol and the whisper.cpp worker binary |
| `crates/models` | Approved model manifest, verified downloader, model store |
| `crates/bench` | Benchmark and soak tooling |
| `vendor/whisper.cpp` | Pinned whisper.cpp (v1.9.5) submodule |

## Documentation

| Document | Contents |
|---|---|
| [docs/SUPPORT.md](docs/SUPPORT.md) | User help, troubleshooting, privacy, uninstall |
| [docs/PLATFORM_SUPPORT.md](docs/PLATFORM_SUPPORT.md) | Per-platform support and requirements |
| [docs/IMPLEMENTATION_STATUS.md](docs/IMPLEMENTATION_STATUS.md) | What is implemented and verified, measurements, open items |
| [docs/RELEASE.md](docs/RELEASE.md) | Release procedure |
| [CHANGELOG.md](CHANGELOG.md) | Version history and versioning policy |
| [docs/00_SPEC_OVERVIEW.md](docs/00_SPEC_OVERVIEW.md) | The original build specification and the reading order of `docs/01`–`docs/12` |
| [DICTATION_MASTER_SPEC.md](DICTATION_MASTER_SPEC.md) | The whole specification in one file |

## License

No license has been chosen for this project yet (`UNLICENSED`), so all rights are reserved by the author. Third-party components keep their own licenses, listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The Whisper model weights are MIT-licensed by OpenAI and are downloaded or imported by the user, not bundled.
