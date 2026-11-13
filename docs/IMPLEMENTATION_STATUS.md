# Implementation status

Last updated: 13 November 2026 · **M0 partially complete · M1–M7 implemented, native GUI runs and release credentials pending** · Version 0.8.0

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

## M7: Linux experiments

The exit criterion is a "published limited support matrix; no broad support claim" (docs/07 §2). **Met**, in `docs/PLATFORM_SUPPORT.md` → Linux. No physical Linux desktop was available, so no desktop or compositor is named as supported.

| Item | Where | Evidence |
|---|---|---|
| **Session probe** | `crates/platform/src/linux.rs` (`probe`) | Reports session type, desktop, X11 reachability, EWMH window manager, whether the global shortcut is reliable, a *running* portal's GlobalShortcuts interface, and logind. Every D-Bus call has a 1 s method timeout. **Finding:** the first version took 120 s on a headless host, because introspecting `org.freedesktop.portal.Desktop` D-Bus-activated the portal in a session with no desktop. The probe now inspects only a portal that is already running. |
| **Window identity (X11)** | `capture_target`, `check_target` | `_NET_ACTIVE_WINDOW`, `_NET_WM_PID`, lower-case `WM_CLASS`, and the process start time from `/proc/<pid>/stat` (PID reuse). The field is always Unknown, so `check_target` never returns `Same`. **Tested under Xvfb:** a simulated EWMH window manager switches the active window and gives `WindowChanged`; the app is identified by `WM_CLASS` and pid. |
| **Copy (X11 CLIPBOARD owner)** | `clipboard_write_text` | A server thread owns `CLIPBOARD` and serves `TARGETS`, `UTF8_STRING`, `TEXT` and `text/plain;charset=utf-8`. It exits on `SelectionClear`. Ownership is confirmed within 500 ms; text is limited to 64 KiB, so INCR is never needed. **Tested under Xvfb:** an exact Unicode round trip read back by a separate X client, and handover to a newer copy. Before M7, Copy did not work on Linux at all. |
| **Sleep, shutdown, lock** | `watch_suspend` | logind `PrepareForSleep(true)`, `PrepareForShutdown(true)`, and `Lock` on the session from `GetSessionByPID`. The watch starts without blocking (tested). Firing it needs a real sleep or lock, which was not done. |
| **Automatic paste** | n/a | **Off by design.** No editable-element identity (docs/05 §4). AT-SPI was not attempted within the time box. |
| **UI** | `src/components/SessionNotice.tsx`, `Snapshot.session` | On Linux the app explains three things: drafts instead of paste; on Wayland, use Start in the window; Copy unavailable without X11/XWayland. These come from the probe, which runs once at startup. |
| **Packaging** | CI `linux-package`, `scripts/verify-linux-deb.sh` | A `.deb` for Ubuntu 24.04: install with apt, run inference through `/usr/bin/dictation-worker`, remove. AppImage, Flatpak and RPM were not attempted. **Result (CI run 43386942361):** an 8.3 MB `Local Dictation_0.7.0_amd64.deb`, depending on `libayatana-appindicator3-1`, `libwebkit2gtk-4.1-0` and `libgtk-3-0`. It installs `/usr/bin/dictation-app` and `/usr/bin/dictation-worker`. The installed worker gives JFK median 1.17 s (RTF 0.11), normalized WER 0%. The package is then removed. |

**Measured on this host (Ubuntu 22.04, tty, no display):** probe `kind: none`, `x11_reachable: false`, `logind: true`. **Under Xvfb:** `kind: x11`, `x11_reachable: true`, `ewmh_active_window: false` (no window manager). **In CI (`ubuntu-24.04`, run 43386942361):** without a display, the probe reports `kind: none` and `logind: true`. Under `xvfb-run`, `x11_clipboard_roundtrip_and_ownership_handover` and `x11_window_identity_changes_are_reported` pass.

## M6: Windows

The exit evidence is "separate Windows release evidence" (docs/07 §2). **Partially met.** Everything below comes from GitHub-hosted `windows-2025` runners: no physical Windows machine, no signing, and no supported-app qualification. Design decisions are in ADR-012; user-facing limits are in `docs/PLATFORM_SUPPORT.md` → Windows.

| Item | Where | Evidence |
|---|---|---|
| **Target identity** | `crates/platform/src/win32.rs` | **Captured:** root foreground HWND; pid with process creation time; lower-case exe name; the UI Automation focused element (runtime id, control type, password, enabled, read-only ValuePattern), which must belong to the foreground process. **UIPI:** a target at higher or unreadable integrity is Unknown. A null foreground window (secure desktop) is not a target. |
| **Clipboard and paste** | same | **Clipboard:** `CF_UNICODETEXT`, with opening retried for up to 500 ms. **Paste:** one `SendInput` Ctrl+V; a partial submission is Uncertain and releases only the synthetic keys. **Modifiers:** `GetAsyncKeyState`. |
| **Native end-to-end test** | `win32.rs` tests (`--ignored`, CI step) | Real Edit controls on the runner's desktop. Unicode (umlauts, em dash, emoji, CJK) goes through the real clipboard and one real Ctrl+V, and is read back from the control and through UIA. The own app is never a target, a field switch inside the window changes the runtime id, and a password field is Secure. **Result:** passes on the `windows-2025` runner (CI run 43355765924). `RECEIVED=true DUPLICATED=false` with the exact text `Grüße — naïve café, 😀 漢字 ok`; the UIA readback matches; the second field stays empty. |
| **Session watch** | `watch_suspend` | A hidden top-level window on its own thread: `WTSRegisterSessionNotification` (lock, logoff, console/remote disconnect), `PBT_APMSUSPEND`, end-session. The test checks that it starts and stops; the events themselves were not fired. |
| **Permissions** | `permission_status`, `open_privacy_settings` | Microphone status comes from the consent store (machine, user, NonPackaged) and the app links to `ms-settings:privacy-microphone`. Accessibility reports Granted because Windows needs no grant at equal integrity. Onboarding text is platform-specific. |
| **Worker** | `crates/transcription-protocol/src/job.rs`, `client.rs`, worker `main.rs`, `build.rs` | **Lifetime:** a `KILL_ON_JOB_CLOSE` job object, tested by killing a child through job close. **Spawn:** `CREATE_NO_WINDOW`, with only `SystemRoot` kept. **Model open:** reparse-point-safe, checked on the opened handle. **Build:** MSVC static-lib discovery, `advapi32`. |
| **Installer** | CI `windows-installer`, `scripts/verify-windows-installer.sh` | NSIS installer, per-user (`currentUser`), with an embedded WebView2 bootstrapper. It is installed silently as the runner user, the installed worker transcribes the JFK fixture, and it is uninstalled. **Result (CI run 43370896636):** a 6.1 MB `Local Dictation_0.6.0_x64-setup.exe`. The silent install takes 2 s to `%LOCALAPPDATA%\Local Dictation` (`dictation-app.exe`, `dictation-worker.exe`, `THIRD_PARTY_NOTICES.md`, `uninstall.exe`) with no elevation prompt; WebView2 156 was already present. The installed release worker gives JFK median 2.9 s (RTF 0.26), normalized WER 0%; cold start median 3.2 s, model load 305 ms. The uninstall removes the program files. The bench does not read process RSS on Windows, so its "0 MiB" peak means *not measured*. |

### M6 findings

1. **On MSVC, whisper.cpp was compiled without optimization.** The `cmake` crate replaces CMake's MSVC `CMAKE_C_FLAGS_RELEASE` with ` -W0`, which drops `/O2 /Ob2 /DNDEBUG`. The 11 s fixture took over 50 s per job even in a release build, and the real-worker tests timed out. `build.rs` now passes the flags explicitly for MSVC. GCC and Clang were not affected: their cache keeps `-O3 -DNDEBUG`.
2. **Shallow CI checkouts carry no submodule tags.** The worker reported a bare hash instead of `v1.9.5`. `build.rs` now names the tag only when HEAD is the pinned commit.
3. **Windows worker timing after the fix** (`windows-2025` runner, portable x86-64-v3, debug Rust with Release ggml): model load 378–393 ms (including the verified-hash read); 11 s fixture warm 3.9–4.3 s (RTF ≈ 0.36); exact transcript, auto-detected `en`.
4. **The test rig, not the app, hung the first native e2e runs.** There were two causes:
   - The test thread owned the Edit controls and also queried UIA. UIA calls are answered by the window-owning thread, so it blocked on itself. The test now queries from another thread while the window thread pumps. The app is not affected: it queries from the delivery thread, which owns no windows, and its targets belong to other processes.
   - A synthetic Alt tap, used to satisfy the foreground-lock rule, made `DefWindowProc` enter modal menu mode (`SC_KEYMENU`) inside `DispatchMessage`. A Shift tap satisfies the rule without that.
   The test now has stage markers and a 120 s watchdog.
5. **Git Bash rewrites POSIX-looking arguments for native programs.** `setup.exe /S` reached NSIS as a drive path, so the installer waited in its GUI until the job timeout. The verifier sets `MSYS_NO_PATHCONV=1` and bounds every phase.
6. **The GitHub `windows-2025` runner reports 2 logical CPUs** (`nproc`), so its timings are not comparable with the docs/08 reference hardware.

**Not done in M6:**

- The supported-app matrix (Notepad, a browser textarea, Word, VS Code) and testing as a standard (non-admin) user on a real machine. The runner account is an administrator.
- Narrator smoke test, Authenticode signing, SmartScreen reputation.
- Windows trigger latency, RSS and idle CPU on reference hardware.

## M5: hardening and release tooling

The exit criteria are "all release gates and clean-machine install" (docs/07 §2). **These are not met.** Two things are missing: a consented release corpus (docs/08 §3) and a Developer ID certificate. A clean-machine install also needs a person at the Mac. M5 delivers the tooling and every gate that can be measured without them, with the results below.

| Item | Where | Evidence |
|---|---|---|
| **Sleep, lock, user switch, logout → `Suspend`** | `crates/platform/src/macos.rs` (`watch_suspend`), wired in `src-tauri/src/lib.rs` | NSWorkspace will-sleep, screens-did-sleep, session-did-resign-active and will-power-off, plus the distributed `com.apple.screenIsLocked` notification. All of them send `Command::Suspend` and stop a running mic test. The reducer already cancels, kills the worker and clears the draft (tested since M1). The native hook is **compiler-checked only**, because sleeping or locking the Mac over SSH isn't possible. |
| **Diagnostics export** | `crates/coordinator/src/diagnostics.rs`, `src-tauri/src/diagnostics.rs`, `src/features/settings/SupportReport.tsx` | **What it records:** bounded rings of outcome codes (64) and job timings (100), the worker's Hello facts, and the number of restarts. **Report:** versions, OS and architecture, protocol, model ids, permission codes, readiness, and settings reduced to non-identifying values (no device names, no excluded-app ids). **UI:** the preview shows exactly the text Save writes, and Save goes through a native dialog opened by Rust. **Tests:** the soak test asserts a dictated token never appears in the report; frontend tests check that Save needs a preview first and handles a cancel. |
| **Tray status** | `update_tray_status` | The tooltip follows the phase and readiness: *Ready*, *Listening…*, *Setup needed: install a model*, and so on. |
| **Coordinator soak** | `crates/coordinator/tests/soak.rs` | 500 sessions through the real actor, DSP and resampler (48 kHz stereo), mixing endpoint finish, stop, cancel while listening, cancel while transcribing (kill), silence and disconnect. **After every session:** exactly one outcome, mic closed. **At the end:** no job ran twice; RSS 8.3 → 8.5 MB, under the 10% growth limit. Passes on Linux and on the M2 Pro. |
| **Benchmark harness** | `crates/bench` (`dictation-bench`) | `corpus`: raw and normalized WER, CER, latency median/p95/max, RTF, 10 cold starts, worker peak RSS. `nospeech`: synthetic no-speech fixtures, accepted cases sent through the real worker. `soak`: the real worker with hard kills every 10th session, an orphan check and an RSS trend. The normalization policy is fixed in `crates/bench/src/lib.rs`: NFC, lowercase, punctuation → space, no number normalization. |
| **Signing and installer** | `src-tauri/Entitlements.plist`, `tauri.conf.json`, `scripts/verify-macos-bundle.sh`, `.github/workflows/release.yml` | Hardened runtime with only `com.apple.security.device.audio-input`. The default build is ad-hoc signed; a Developer ID signature and notarization are used when the `release` environment secrets exist. **Verified on the M2 Pro:** a headless `CI=true pnpm tauri build --bundles app,dmg` gives a 6.2 MiB DMG; `hdiutil verify` reports a valid checksum; `codesign --verify --deep --strict` passes; app and nested worker both carry `adhoc,runtime`; the entitlements are exact. The **signed, hardened worker inside the bundle runs inference**: the corpus benchmark below used it. |
| **SBOM and notices** | `scripts/notices.mjs`, `THIRD_PARTY_NOTICES.md` | It walks the normal (shipped) dependency graph of the app and worker on all three release targets: 420 crates, 4 npm packages, whisper.cpp/ggml and the model weights. License texts are grouped where identical. A CycloneDX 1.5 SBOM comes out at release time. **Policy:** permissive licenses only, enforced in CI (`pnpm notices:check`); 0 violations. The notices file is bundled as an app resource. |
| **Docs** | `docs/SUPPORT.md`, `docs/RELEASE.md` | User help covers microphone, model, language, silence, permissions, shortcut conflicts, the reasons a draft is kept instead of a paste, troubleshooting, privacy, and uninstall with data locations. The release procedure covers the secrets, the docs/10 §3 checklist and rollback. |

### M5 measurements

These are **smoke measurements**, not release evidence. The corpus is a single public-domain English clip, and the no-speech set is synthetic.

| Metric | Linux i3-10100 (native build) | Apple M2 Pro (signed bundle worker, CPU, portable) |
|---|---|---|
| JFK 11 s, 30 warm runs: median / p95 / max | 990 / 1688 / 3899 ms | 376 / 605 / 614 ms |
| RTF (median) | 0.090 | 0.034 |
| WER raw / normalized / CER | 4.5% / 0.0% / 0.0% | 4.5% / 0.0% / 0.0% |
| Cold start (spawn + verified load + first job), 10 runs: median / p95 | 1682 / 2825 ms | 533 / 771 ms |
| Model load alone (median) | 716 ms | 140 ms |
| Worker peak RSS (Base, incl. model) | 242 MiB | 244 MiB |
| No-speech suite (29 synthetic cases): endpointer accepts | 6 / 29 | 6 / 29 |
| No-speech suite: automated insertions (non-empty text) | **0 / 29** (95% upper bound 9.8%) | **0 / 29** |
| DSP time per 10 ms block: median / p95 | 9.7 / 14.2 µs | 6.0 / 6.4 µs |
| Real-worker soak, 500 sessions | 1590 s; 451 jobs, 49 hard kills + respawns, **0 orphans**; 3 s clip median 1050 / p95 11249 ms (host shared with other users, load average about 18 on 8 threads, so latency is not meaningful); worker RSS flat at about 245 MB (trend +0.0%) | 174 s; 451 jobs, 49 hard kills + respawns, **0 orphans**; 3 s clip median 317 / p95 501 ms; worker RSS flat at about 248 MB (trend −0.0%) |

The raw WER of 4.5% is a single token: the reference has "so," and the hypothesis has "so".

### M5 findings

1. **The VAD accepts loud broadband noise and sustained tones.** All five −25 dBFS white-noise seeds and a sustained three-note chord passed the endpointer as speech: 208–384 ms of speech for the noise, 7.1 s for the chord. In every case whisper returned empty text, so the session ends as *no usable text* and nothing is inserted. The thresholds were **not** retuned on this set. docs/08 §3 requires tuning on a separate tuning set; this synthetic suite is regression material, not that set.
2. **The bundle identifier is still `app.localdictation.m0`.** Changing it moves the data directory and resets macOS permission grants, so it needs a product decision before the first public release. Recorded in docs/RELEASE.md §5.

**Not done in M5:**

- Developer ID signing, notarization and Gatekeeper acceptance: no certificate.
- The release corpus and the docs/08 §4 quality gates: no consented recordings.
- Clean-machine install and permission evidence, and the 24-hour tray residency test: these need a person at the Mac.
- Trigger latency, idle CPU and parent + worker RSS of the running app (docs/08 §5): these need the GUI app.
- Reviewed German and Indonesian UI translations, and an adjustable HUD position.

## M4: product UI

The exit criteria are "keyboard-only setup, permission-denial recovery and trial" (docs/07 §2). They are covered by frontend tests in jsdom (vitest + Testing Library) and by macOS builds. VoiceOver and real permission prompts still need a person at the Mac.

| Item | Where | Evidence |
|---|---|---|
| **Permissions (macOS)** | `crates/platform/src/macos.rs` | Microphone: `AVCaptureDevice authorizationStatusForMediaType` / `requestAccessForMediaType:completionHandler:`. Accessibility: `AXIsProcessTrustedWithOptions` with the prompt option. Deep links to the Privacy panes via `NSWorkspace openURL`. The status is read from the OS on every check and never cached (docs/04 §6). Other platforms report `notApplicable`. |
| **Onboarding** | `src/features/onboarding/` | Steps follow docs/02 §2: welcome with the clipboard disclosure → microphone (request on Continue, denial → Settings + Check again, test stops on navigation) → model → shortcut (registration must succeed) → automatic paste or copy-only → trial → done (launch at login, off by default). A step with a requirement can't advance until it is met. Focus moves to each step heading. |
| **Shortcut capture** | `src/chord.ts`, `ShortcutField`, `set_shortcut` | Layout-independent `KeyboardEvent.code`. It needs a modifier other than Shift alone, and plain Tab still navigates. The global shortcuts pause while the field is focused. The chord is saved only after the OS registers it; on failure the previous chord is restored and a conflict message is shown. |
| **Settings pages** | `src/features/settings/SettingsApp.tsx` | General, Recording, Shortcuts, Transcription (with Models and Unload), Delivery (mode, exclusions, Accessibility status and enable flow), Privacy (data lifecycle) and About (versions, licenses, engine self-test) per docs/02 §3. A vertical ARIA tablist with arrow-key navigation. |
| **Review window** | `review.html`, `src/review/`, `capabilities/review.json` | A separate window that deliberately takes focus. Its capability allows **only** get/edit/copy/discard draft. Opening it revokes automatic delivery for an in-flight session. It shows a retention countdown (`expires_in_ms` from the coordinator). Rust also checks the calling window's label for draft commands (docs/06 §1). |
| **HUD** | `src/hud/`, `TauriNotifier` | Shows phase, level, and elapsed time while listening or transcribing. A 3.5 s banner (no text) appears on Text ready, No speech or failure; a sequence counter means a delayed hide never hides a newer session's HUD. |
| **Config schema v2** | `crates/domain/src/config.rs` | Adds `ui.onboarding_complete` and `ui.reduced_motion`. The v1 → v2 migration is tested and keeps the revision and all settings. Users of earlier builds see onboarding once. |
| **Launch at login** | `tauri-plugin-autostart` 2.7.0 | A per-user LaunchAgent, opt-in only. No Apple Events automation. |
| **Accessibility and i18n** | `src/i18n.ts`, styles | Meters are `aria-hidden` with a stable label. Phase and outcome text use `role="status"`; meter values are never announced. Visible focus rings, reduced motion, and a narrow-window layout. Onboarding strings come from an English catalog; German and Indonesian catalogs exist and fall back to English until reviewed translations exist. |
| **Tests** | `src/**/*.test.ts(x)`, CI frontend job | 13 tests. Onboarding: focus moves to step headings with Tab/Enter only; the microphone step can't be skipped; recovery after denial (Settings + Check again); the model is required. ShortcutField: pause/resume, invalid chords, OS conflict not reported as success, Tab still navigates. Plus chord parsing/display and the completeness of the docs/02 §5 copy. |

**Not done in M4:**

- Adjustable HUD position.
- Readiness and mode shown in the tray menu.
- Redacted diagnostics export (docs/06 §6).
- Reviewed German and Indonesian UI translations.
- A sleep/lock/logout hook calling `Input::Suspend`. This is required by docs/04 invariant 10 and is tracked for M5 hardening.

**Flaky test seen once:** a full `cargo test` run on the Linux host aborted with 1 failure (44 passed) right after a rebuild, and four repeated runs passed. A timing-sensitive test under CPU load is suspected; it hasn't been identified yet.

## M3: delivery vertical slice

The exit criteria are "one complete shortcut-to-text flow and focus-change/cancel tests" (docs/07 §2). They are met with a scripted platform through the real actor, reducer and delivery adapter. They are **not yet proven against TextEdit or a browser on the Mac**: that's the M0 T0.5/T0.6 checklist, which needs a person and the Accessibility grant.

| Work package | Where | Evidence |
|---|---|---|
| **C1:** target and permission adapters | `crates/coordinator/src/delivery.rs` (`TargetPlatform`, `NativePlatform`), `crates/platform` | At the start press, a target qualifies only if the field is Editable, it isn't the app itself, it isn't excluded and it re-checks as Same. Otherwise the session is copy-only. Unknown, secure, not-editable and uncapturable targets (Accessibility not granted) never auto-paste. |
| **C2:** delivery transaction | `PlatformDelivery` | Runs on its own thread because macOS AX references aren't `Send`. Steps: re-check, then wait ≤1500 ms for physical modifier release (never synthesized), then plain-text clipboard write, then re-check again, then `DeliveryPrepared`. The reducer commits the ledger, then exactly one paste chord is posted. `release` sets a cancel flag that the transaction checks before writing the clipboard. |
| **C3:** full vertical slice | `crates/coordinator/tests/delivery.rs` | 15 tests, covered below. |
| App | `src-tauri/src/lib.rs`, settings form | `PlatformDelivery<NativePlatform>` replaces `CopyOnlyDelivery`. Delivery mode and excluded apps apply live. The opt-in paste-draft chord (`Ctrl+Option+V`) is re-registered when it changes, and the previous chord is released. |

**What the C3 tests cover:**

- **Happy path:** one paste into the original target with the text staged, then a second session.
- **Focus changes:** app switch and field switch during transcription each end in a draft, with the clipboard untouched.
- **Unusable targets:** secure, unknown, not-editable and uncapturable targets are copy-only and never write the clipboard on their own. Terminals and user exclusions are copy-only.
- **Modifiers:** keys held past the timeout end in a draft with no clipboard write; keys released within the timeout still paste.
- **Clipboard:** a busy clipboard means no input. If focus is lost after the clipboard write, there's no input, and the clipboard state is reported.
- **Cancel and send:** a cancel during the modifier wait never touches the clipboard. An uncertain send is attempted exactly once and gives an attempted draft.
- **Explicit paste:** it goes to the newly focused field and consumes the draft. Into a secure field it is refused and the draft is kept.
- **Settings and stress:** the copy-only setting works with Copy. Across 12 sessions with cancels there is never a double dispatch.

**Added error code:** `MODIFIERS_HELD`. The spec says to keep a draft when the modifiers are still held after 1.5 s (docs/05 §4 step 5), but its AppError list has no code for that.

**Not done in M3:**

- Accessibility is only *reported*. The explicit "Enable automatic paste" flow that prompts via `AXIsProcessTrustedWithOptions` and links to System Settings is M4 onboarding.
- Clipboard preservation is P1 (docs/05 §6).
- No Windows or Linux delivery backend yet.

## M2: local engine and models

The exit criteria are "offline known fixture transcribed; malformed protocol rejected; no orphan worker" (docs/07 §2). They have been met since M0 by the worker tests, on Linux and on the M2 Pro. M2 adds the model manager (B4) and verified activation.

| Item | Where | Evidence |
|---|---|---|
| Approved manifest | `crates/models/src/manifest.rs` | Base (147,951,465 B, `60ed5bc3…`) and Small (487,601,967 B, `1be3a9b2…`) multilingual. URLs pin the Hugging Face repo revision `5359861c…`. Both hashes were verified by full independent downloads, not only taken from the server's metadata (docs/06 §5). |
| Downloader | `crates/models/src/download.rs` | Fake-server tests cover: HTTPS only; allowlisted hosts (`huggingface.co`, `*.hf.co`) checked on every redirect hop; ≤5 redirects; size cap enforced while streaming and against Content-Length; resume only with an exact `Content-Range`, otherwise a restart from 0; hash mismatch or oversize deletes the partial; cancel and transient errors keep it. The real network path (ureq 3.4.2 + rustls, no gzip) installed Base in 3.0 s on the Linux host. |
| Store | `crates/models/src/store.rs` | `.partial` files never count as installed. Atomic rename after fsync. The `.verified` marker binds hash, size and mtime. Symlinked model files are Invalid. Import copies a user-selected regular file with the same checks. All files are 0600 in a 0700 directory; a resumed partial is tightened too, a gap the tests caught. |
| Single-pass verified load | `crates/transcription-worker/src/{main,engine}.rs`, `shim.c` | whisper.cpp loads through `whisper_init_with_params` with a custom loader over one `O_NOFOLLOW` descriptor that is hashed as it's read. The context is dropped on a mismatch. This closes the M0 hash-then-reopen window and halves model I/O. |
| Activation | `WorkerTranscriber::switch_model`, `Command::SwitchModel` | The new model loads in a **fresh** worker, runs a 1 s smoke transcription, and only then is swapped in and saved to settings. Real-worker test: a tampered hash is refused and the previous model keeps transcribing; a valid switch moves to a new epoch; Unload makes the next session fail with ModelUnavailable. Switching is refused during a session, and starting a session is refused during a switch. |
| App | `src-tauri/src/models.rs`, `src/features/models/` | Commands: `get_models`, `install_model`, `import_model`, `verify_model`, `cancel_model_operation`, `remove_model`, `activate_model`, `unload_model` (main window only, manifest ids only). The import picker is opened by Rust; webviews get no dialog or fs permissions. One operation per model. The active model can't be removed. The first installed model is activated automatically. Progress events are capped at 10 Hz. |

**Not done in M2:**

- Windows job-object child lifetime (M6).
- Model RSS measurements (docs/08 §5).
- Small has not been benchmarked against Base yet, so docs/08 §4 quality gates are still open.

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

- **Delivery was copy-only in M1.** M3 replaced it with `PlatformDelivery`.
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
8. **The worker rejects symlinked model paths.** The M0 window between hashing and re-opening the file was closed in M2 by the single-pass verified load described above.

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
./scripts/fetch-model.sh                     # test fixture model for cargo test (the app installs its own via Settings → Models)

cargo test                                   # all 46 tests, incl. real inference
cargo run -p dictation-audio --example mic_probe -- --seconds 8         # T0.2
cargo run -p dictation-platform --example m0_target_probe               # T0.5
cargo run -p dictation-platform --features m0-diagnostics --example m0_paste_probe   # T0.6

PROFILE=debug ./scripts/stage-worker.sh
pnpm tauri dev                               # T0.3/T0.4/T0.7 in-app (install a model under Settings → Models)
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
- [ ] **M4 onboarding:** a fresh start (or settings reset) shows onboarding. Complete it **with the keyboard only**: the microphone prompt appears only after Allow, denial → Open System Settings → Check again works, a model download shows progress, a conflicting shortcut is refused, Accessibility via "Enable automatic paste" → Check again, and the trial shows recognized text.
- [ ] **M4 VoiceOver:** step headings are announced, the meter is not announced continuously, and phase changes are announced once.
- [ ] **M4 review window:** tray → Review text… shows the draft with a countdown; Copy and Discard work; the HUD banner appears on Text ready.
- [ ] Accessibility denied → target capture returns `NotTrusted` and nothing is pasted

## Not in M0 (next: M1)

Coordinator actor and session state machine, generated TS bindings, config persistence, the real start/stop dictation flow, the model manager (download/import), onboarding, and Windows/Linux adapters. The worker self-test embeds the JFK fixture via `include_bytes!`; remove it or compile-gate it before the product build.
