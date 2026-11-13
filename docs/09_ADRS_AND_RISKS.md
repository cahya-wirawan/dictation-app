# 09 — Architecture decisions and risk register

## ADR-001 — Rust owns the dictation lifecycle

Accepted. Native capture/shortcuts/target tracking must survive a hidden or reloaded webview. Rust is the authority; TypeScript is presentation. Alternative browser MediaRecorder/Web Audio simplifies demos but introduces a second capture/permission stack and webview lifecycle coupling. Consequence: more Rust and OS adapter work, simpler product invariants.

## ADR-002 — Pinned whisper.cpp in a persistent native worker

Accepted for v1. CPU-capable local engine, one warmed model, inherited private pipes. Native worker contains crashes and supports kill-based cancellation. It does not constitute an OS sandbox. Alternative in-process FFI is smaller but cancellation/crashes affect the main app. Python/faster-whisper can be a capable local alternative but adds runtime packaging and is outside this Rust-focused distribution. A future engine must implement the same bounded protocol and pass the same corpus/delivery tests.

## ADR-003 — Phrase finalization rather than streaming insertion

Accepted. Transcribe once after explicit stop or silence. Users see a listening/processing indicator but no unstable partial text injected into other apps. Alternative continuous streaming requires revision/deduplication semantics and makes correction of already inserted text difficult. Consequence: latency includes endpoint wait and phrase inference; v2 can add review-only partials without changing automatic delivery semantics.

## ADR-004 — Learned/classifier VAD plus endpoint policy

Proposed, gated by M0. WebRTC VAD at a valid fixed frame size is the first candidate; RMS alone is not adequate for speech/noise distinction. Silence detection is a separate policy around speech classification. Alternative Silero integration can improve robustness but adds a model/runtime dependency. Consequence: use a detector trait and golden endpoint fixtures; replacement must preserve sample/timing contracts.

## ADR-005 — Clipboard plus native paste chord

Accepted. Unicode clipboard transfer is more dependable than typing each character under arbitrary keyboard layouts. Native clipboard and key APIs remain platform-specific. Alternative Accessibility text setting can bypass key synthesis in some apps but varies across editors and can replace text incorrectly; reserve for explicit tested adapters. Default leave-text policy is disclosed; full restoration is not promised. Copy-only is a first-class workflow.

## ADR-006 — Conservative original-target validation

Accepted. Auto-paste requires a live original app/window and adequate focused editable-element evidence. Unknown identity becomes a draft. Do not force focus back after a user switches apps. Consequence: more manual fallback in unsupported editors, fewer wrong-destination opportunities. This is a usability/safety tradeoff, not a guarantee against all focus races.

## ADR-007 — macOS first, explicit support tiers

Accepted planning default. Platform complexity is concentrated in permissions, target probing, clipboard and non-activating UI. First qualify Apple Silicon macOS; Windows follows. Linux Wayland automatic paste is uncommitted pending compositor/portal proof. Consequence: shared architecture is portable, but release scope remains evidence-based.

## ADR-008 — No history or cloud rewriting

Accepted. Raw audio is transient; drafts live in memory with TTL. No LLM cleanup means words are not silently paraphrased and no second heavyweight model is needed. Alternative optional history and rewrite modes require new privacy/storage/product decisions, not a hidden incremental feature.

## ADR-009 — Base multilingual candidate, not an assumed universal default

Proposed. English-only models cannot meet multilingual scope. Base is a useful latency/size starting point; Small is a quality candidate. Final model/profile must satisfy held-out language gates and reference-hardware budgets. CPU is baseline; Metal is an optional measured improvement. No invented speed/accuracy claims.

## ADR-010 — earshot neural VAD on 16 ms frames (resolves ADR-004 candidate)

Accepted for M1, pending corpus calibration (12 November 2026, M0 T0.8). The WebRTC VAD candidate failed the maintenance check: the `webrtc-vad` crate's last release is 0.4.0 from October 2019. Replacement: `earshot` 1.2.2 (MIT OR Apache-2.0, pure Rust, no ONNX runtime, ~8 KiB state per detector, released August 2026). It needs 256-sample (16 ms) frames at 16 kHz instead of the 20 ms/320-sample frames assumed in 03 §4, so the endpointer counts 16 ms frames and converts every millisecond setting with ceiling division: pre-roll 300 ms → 19 frames, silence 1000 ms → 63 frames, trailing 200 ms → 13 frames. The onset rule stays three voiced frames within five, which shortens the onset window from 100 ms to 80 ms. A frame counts as voiced only when the score is ≥0.5 and RMS is ≥−50 dBFS. M0 measurements (synthetic plus one public-domain clip, not a corpus): voiced fraction 0.675 on jfk.wav, 0.000 on digital silence, −55 dBFS hiss and a 1 kHz tone, 0.059 on −25 dBFS white noise. Consequence: `SpeechDetector` stays the swap point. Silero or whisper.cpp VAD remain alternatives if the held-out no-speech suite fails.

## ADR-011 — Capture always binds a specific device id

Accepted (M0 T0.2). CPAL 0.18 transparently reroutes a stream opened on the *default* device when the OS route changes (`ErrorKind::DeviceChanged`). That would be a silent microphone substitution, which 11_AGENT_HANDOFF forbids. Capture therefore resolves the current default to its concrete `DeviceId` and opens that device. Disconnect, reroute or xrun is a fault that aborts the session; it never transcribes a truncated recording. Consequence: an OS default-device change during a session ends the session with MIC_DISCONNECTED, and the next session uses the new default.

## ADR-012 — Windows target identity, UIPI and CPU baseline

Accepted (M6). The Windows adapter (`crates/platform/src/win32.rs`) follows docs/05 §3–§4 with these concrete choices:

- **Target token.** Root foreground HWND, process id plus process creation time (PID reuse), and the lower-case executable name as the app identity. The focused element is identified by its UI Automation runtime id, and it must belong to the foreground process. A changed runtime id means FieldChanged. A missing element, or one owned by another process, is Unknown.
- **Field class.** Password elements are Secure. Disabled elements, read-only ValuePattern elements and controls that are not Edit or Document are NotEditable. Only Edit and Document controls qualify.
- **UIPI.** SendInput into a higher-integrity process fails without an error. A target whose token integrity is above the app's, or cannot be read, is therefore Unknown and never auto-delivered. A null foreground window (secure desktop, UAC, lock screen) is not a target. The app never requests elevation.
- **Permissions.** Windows has no Accessibility-style grant for same-integrity UIA and SendInput, so `accessibility` reports Granted. Microphone status comes from the capability consent store (machine, user, NonPackaged). There is no runtime prompt, so the app links to `ms-settings:privacy-microphone`.
- **Worker lifetime.** The worker is assigned to a `KILL_ON_JOB_CLOSE` job object, so it dies with the app even after a crash. It is spawned with `CREATE_NO_WINDOW`, and `SystemRoot` is its only environment variable.
- **CPU baseline.** The portable build uses x86-64-v3 (AVX2, FMA, F16C, BMI2; Haswell 2013 and later), from M0 finding 7. Older CPUs are unsupported rather than silently slow (about 11× slower without AVX2). Runtime CPU dispatch (`GGML_BACKEND_DL`) remains a later option.
- **Installer.** NSIS, per-user (`currentUser`, no admin), with an embedded WebView2 bootstrapper. Machines without WebView2 need internet once. Windows 11 and current Windows 10 builds ship WebView2.

## ADR-013 — Universal macOS build (Intel Macs)

Accepted. The macOS release is a universal binary: arm64 + x86_64. Apple Silicon stays the priority platform and the qualification reference (docs/07, docs/08); the Intel slice adds reach without changing behavior.

- **Build.** `tauri build --target universal-apple-darwin`. The worker is built once per architecture and joined with `lipo` (`scripts/stage-worker.sh`). Each whisper.cpp build sets `CMAKE_OSX_ARCHITECTURES` explicitly to one architecture.
- **Intel CPU baseline.** The same x86-64-v3 baseline as ADR-012 (AVX2, FMA, F16C, BMI2). Every Intel Mac that runs macOS 13 or later is a 2017 or newer model with AVX2, so this excludes no supported Mac.
- **Inference.** CPU on both slices, as on Apple Silicon. Metal stays off (M1 finding).
- **Evidence.** CI checks that the app and the nested worker contain both slices, and runs the x86_64 worker slice on the JFK fixture under Rosetta 2. Rosetta proves the code is correct, not how fast it is on Intel. Rosetta translates AVX2 only from macOS 15, so the check needs a macOS 15 or later host. Field report: on a MacBook Pro 2019 (2.6 GHz 6-core i7) with Whisper Base, automatic paste into Notes worked, and the text appeared about 1–2 s after the user stopped speaking. Measured Intel timings and formal qualification are still open.
- **Cost.** The DMG roughly doubles in size, because the worker and app binaries carry two slices.

## Risk register

| Risk | Likelihood / impact | Mitigation and owner role |
|---|---|---|
| Non-activating HUD steals focus | Medium / high | Native spike first; tray-only fallback; platform engineer |
| App lacks reliable editable element identity | High / high | Mark unsupported for auto-paste; explicit draft delivery; platform engineer |
| Focus changes between final check and send | Low–medium / high | Minimize interval, no forced focus, clear limitation; platform engineer |
| Short phrase/noise hallucinations | Medium / high | VAD, held-out noise suite, calibrated rejection; speech engineer |
| Indonesian/German Base quality insufficient | Medium / medium | Test Small; explicit language; publish limitations; speech engineer |
| Windows elevation blocks injection | High in elevated targets / medium | Unsupported target state, Copy; never auto-elevate; platform engineer |
| Wayland global input unavailable | High / medium | Portal capability probes, manual mode, scoped compositor support; platform engineer |
| Worker engine/binding API changes | Medium / medium | Pin source/header/build; protocol adapter; core engineer |
| Model/engine licensing issue | Low–medium / high | Asset-specific license review, notices, SBOM; release owner |
| Clipboard managers retain text | High / medium | Product disclosure and copy-only choice; product owner |
| macOS distribution permissions differ from dev | Medium / high | Signed clean-machine testing early; release owner |
| Cancellation waits on uninterruptible native call | Medium / medium | Kill worker, wait/reap, epoch fence; core engineer |
| Huge model/invalid pipe frame exhausts memory | Medium / high | Hash/size/sample caps and worker limits; core engineer |

## Decisions to close before first implementation freeze

These are engineering spikes; the coding agent should investigate and document outcomes without blocking on routine user approval:

1. Exact minimum OS versions and target architectures, matched to tested Tauri/system-webview prerequisites.
2. Maintained compatible VAD/resampler libraries, native build flags and licenses.
3. Actual engine revision and Rust FFI approach, including Metal support and CPU fallback.
4. Focused-element evidence for the initial supported app list and behavior of secure fields.
5. Real model profile latency, accuracy and RSS; final default model.
6. Modifier-release observation that does not require broad key monitoring.
7. Release identity, code-signing/notarization credentials availability and installer formats.

Changing OS scope, adding cloud transcription, persistent history or telemetry is a product decision requiring an explicit new requirement rather than an engineering convenience.
