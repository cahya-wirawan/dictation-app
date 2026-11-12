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
