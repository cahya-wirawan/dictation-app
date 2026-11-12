# 11 — Coding agent handoff

## Mission

Implement this specification as a native desktop dictation app using Rust, Tauri 2 and TypeScript. Start with macOS Apple Silicon. Do not build a browser demo as the deliverable. Do not replace local inference with a cloud endpoint or require Python for end users.

## Read first

Read PRD, architecture, state/contracts, platform/delivery and security documents before scaffolding. Record actual dependency pins and feasibility evidence. Treat target validation, cancellation, bounded memory, private worker IPC and offline operation as foundational behavior.

## First implementation task

Create a Tauri 2 workspace and an M0 spike with:

1. TypeScript/React/Vite settings stub, tray menu and explicit Quit.
2. Rust registered toggle/cancel chords with repeat filtering.
3. Native CPAL capture to a bounded buffer and visible level meter while active only.
4. Non-activating HUD test plus original app/window/field identity probe.
5. Bundled CPU worker linking a pinned whisper.cpp revision; transcribe a licensed fixed fixture over bounded inherited pipes.
6. A development-only fixed-text delivery test to TextEdit using the clipboard plus one paste chord; remove or compile-gate this diagnostic path from release commands.
7. `docs/IMPLEMENTATION_STATUS.md` with exact versions, tested hardware/OS, outcomes and unresolved feasibility issues.

Do not combine the fixed-text diagnostic with arbitrary input-injection IPC. It is a controlled native spike. Do not claim paste correctness from a successful API return alone: inspect receiving text in the fixture target.

## Implementation order

After M0, follow the dependency graph in the implementation plan. Build a real vertical slice before polishing the full settings UI. Keep domain state and endpointing deterministic. Centralize side effects in adapters and coordinator. Use typed messages and worker identity fencing. UI code cannot decide to paste.

## Non-negotiable invariants

- No idle microphone capture; pre-roll exists only after explicit activation.
- No frontend raw keyboard injection, process execution, unrestricted file read or arbitrary URL download API.
- No automatic paste on changed/unknown target; no forced refocusing.
- No automatic retries after dispatch commit.
- Cancel invalidates pending results before cleanup.
- No transcript/audio in logs, persistent history, analytics or network requests.
- No silent microphone substitution after disconnect.
- No unsupported-platform promises or fabricated performance claims.

## Validation and reporting

For each milestone report changed behavior, evidence and limitations. Run relevant tests once changes are complete; broaden when new failures or risks require it. Commit lockfiles and generated contract artifacts. Include a short manual-native checklist whenever UI automation cannot cover platform permissions/focus.

If a dependency or API differs from this design, inspect the current official sources, document the change, and preserve the requirement. Do not guess method names from this prose. Example schemas here are design contracts, not verified compilable API calls. Never embed signing credentials in source.

## Suggested completion report

State which platforms and app versions were actually tested; whether inference is fully offline; which model/backend passed; what happens on focus changes and cancel; actual latency/RSS/quality measurements; and where installers/source live. Label unfinished work clearly. A compiled window or a mocked transcription is not completion.

## Ready-to-use initial prompt

> Read the documents in this kit. Implement milestone M0 first, then continue through the macOS vertical slice and release gates in dependency order. Use Rust for native behavior and Tauri 2 with TypeScript for UI. Pin actual compatible versions, keep inference local using the bundled whisper.cpp worker, and preserve the cancellation, target validation and privacy invariants. Record measurements and native platform evidence. If automatic delivery is unsupported, implement the specified draft/Copy fallback and report the precise limitation. Do not claim success until the receiving application contains the expected test text and the adversarial cancel/focus tests pass.
