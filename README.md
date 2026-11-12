# Local Dictation — Native Desktop Build Kit

Version 1.0 · 12 November 2026 · Status: implementation specification, not an implemented or benchmarked application.

Build a small tray/menu-bar dictation utility using Rust, Tauri 2, and a TypeScript UI. A global shortcut starts microphone capture. A second shortcut press or sustained silence finishes recording. A bundled local Whisper engine transcribes it; the app pastes the result into the original focused application when the target is still valid, or retains a draft for explicit delivery.

## Recommended product decisions

- Rust owns capture, shortcuts, session state, model management, transcription coordination, and text delivery. TypeScript renders onboarding, settings, status, and drafts.
- Tauri 2 + Vite + React + TypeScript. No web server, Python runtime, cloud inference, account, or browser microphone APIs.
- A persistent, bundled native worker links a pinned whisper.cpp build. Parent/worker communicate through private inherited pipes; there is no localhost HTTP service. CPU inference is mandatory; Metal acceleration is a gated optimization.
- Phrase-at-a-time transcription in v1. Toggle-to-dictate, automatic stop after silence, and explicit stop are mandatory. Push-to-talk is a later enhancement.
- Base multilingual Whisper is the initial balanced candidate; small multilingual is an optional quality profile. Final defaults require the prescribed measurements, especially for German and Indonesian.
- macOS Apple Silicon first, Windows x64 second, Linux X11 experimental third. Linux Wayland offers manual workflow until a specific compositor/portal integration is tested. These are release priorities, not a request to change the specified stack.
- No persistent transcript/audio history by default. One draft may remain in memory for recovery. Clipboard exposure is clearly explained.

## Reading order

| File | Purpose |
|---|---|
| [01_PRD.md](docs/01_PRD.md) | Scope, behavior, requirements, acceptance criteria |
| [02_UX_AND_FLOWS.md](docs/02_UX_AND_FLOWS.md) | Screens, keyboard behavior, copy, accessibility |
| [03_ARCHITECTURE.md](docs/03_ARCHITECTURE.md) | Runtime components, threading, audio and model pipelines |
| [04_STATE_AND_CONTRACTS.md](docs/04_STATE_AND_CONTRACTS.md) | Session machine, IPC, worker protocol, configuration |
| [05_PLATFORM_AND_DELIVERY.md](docs/05_PLATFORM_AND_DELIVERY.md) | Permissions, target tracking, clipboard and paste algorithms |
| [06_SECURITY_AND_PRIVACY.md](docs/06_SECURITY_AND_PRIVACY.md) | Threat model, data lifecycle, trust boundaries |
| [07_IMPLEMENTATION_PLAN.md](docs/07_IMPLEMENTATION_PLAN.md) | Dependency-ordered work packages and completion gates |
| [08_TEST_AND_BENCHMARK_PLAN.md](docs/08_TEST_AND_BENCHMARK_PLAN.md) | Fixtures, integration tests, performance and release gates |
| [09_ADRS_AND_RISKS.md](docs/09_ADRS_AND_RISKS.md) | Decisions, alternatives, risks and unresolved spikes |
| [10_RELEASE_AND_OPERATIONS.md](docs/10_RELEASE_AND_OPERATIONS.md) | Packaging, supply chain, support and release process |
| [11_AGENT_HANDOFF.md](docs/11_AGENT_HANDOFF.md) | Instructions and first tasks for a coding agent |
| [12_SOURCES.md](docs/12_SOURCES.md) | Official references and evidence boundaries |

## Authority and changes

PRD defines product behavior. State/contracts define ordering and invariants. Platform document defines delivery safeguards. If these conflict, stop the affected work and record a small ADR; do not silently weaken a safeguard. Numerical defaults are proposals and performance values are targets, not measured results. Source references [S1]–[S13] distinguish external platform facts from our implementation choices. Dependency versions must be pinned during the first engineering milestone; this kit deliberately does not fabricate a tested compatibility matrix.

## Definition of a successful first release

On a clean supported Mac, the user can grant microphone and Accessibility access, install or import a model, press a configured shortcut in a text editor, dictate, wait for silence, and receive one text insertion without the dictation app taking keyboard focus. Offline operation, cancellation, focus-change fallback, permission denial, and Unicode delivery must pass before release.
