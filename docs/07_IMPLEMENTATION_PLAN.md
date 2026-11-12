# 07 — Implementation plan and backlog

## 1. Delivery strategy

Build the uncertain native behaviors first, then a complete thin vertical slice, then polish. Estimates below assume one experienced Rust/TypeScript engineer with physical access to target machines. They are rough engineering effort ranges, not delivery commitments; OS permission/signing issues may dominate. First Mac release: approximately 5–8 engineering weeks after a successful feasibility spike. Windows qualification: approximately 2–3 additional weeks. Do not sum overlapping work mechanically.

## 2. Milestones

| Milestone | Effort | Work | Exit evidence |
|---|---|---|---|
| M0 — Feasibility and pins | 3–5 days | Exact toolchains; capture; native HUD; shortcut; target element; paste; worker build; signing experiment | Spike report, pinned dependency matrix, native target and HUD evidence on release-like build |
| M1 — Domain and capture | 4–6 days | Actor, config, session IDs, CPAL ring, resampler, endpoint detector | Deterministic state/audio fixtures and no idle capture |
| M2 — Local engine and models | 4–6 days | Worker protocol, pinned engine, model install/import, CPU inference, cancellation | Offline known fixture transcribed; malformed protocol rejected; no orphan worker |
| M3 — Delivery vertical slice | 4–6 days | Target token, policy, clipboard, one-shot paste, draft recovery | One complete shortcut-to-text flow and focus-change/cancel tests |
| M4 — Product UI | 4–6 days | Onboarding, settings, model progress, HUD, review, accessibility | Keyboard-only setup, permission-denial recovery and trial |
| M5 — Hardening and release | 5–8 days | Corpus benchmarks, soak, signing, installer, diagnostics, docs | All release gates and clean-machine install |
| M6 — Windows | 8–15 days | Native adapter, permission UX, packaging, supported-app matrix | Separate Windows release evidence |
| M7 — Linux experiments | Time-boxed 3–5 days | Named desktop/compositor probing, manual fallback, packaging research | Published limited support matrix; no broad support claim |

## 3. M0 task breakdown

- T0.1 Scaffold Tauri 2 with TypeScript/React/Vite, strict lint/typecheck, Cargo workspace and lockfiles. Pin actual versions in a compatibility document. Confirm OS minimums and physical test machines.
- T0.2 Capture 48/44.1 kHz microphone audio in Rust and prove selected-device permission behavior in a signed app bundle.
- T0.3 Register toggle/cancel chords, filter repeat, detect modifier release without keyboard-content monitoring.
- T0.4 Show a HUD without changing the frontmost app, window, focused field or caret on repeated trials. If impossible, use tray-only and record the reduced UX.
- T0.5 Probe editable element identity in TextEdit and a browser textarea. Show focus changes within the same window are caught or explicitly unsupported.
- T0.6 Stage Unicode clipboard text and dispatch one native paste; inspect actual receiving document in the test harness.
- T0.7 Build whisper.cpp CPU worker, load Base multilingual model, measure a 10 s fixture and validate kill/restart. Verify target naming/signing of bundled executable [S5].
- T0.8 Validate VAD binding maintenance/license/frame requirements and resampler behavior. Freeze candidate or write replacement ADR.

Stop cross-platform auto-paste work if target validation is unprovable on that platform. Continue capture/transcription/copy-only independently; do not pretend the blocker disappeared.

## 4. Work packages

| ID | Depends on | Deliverable | Acceptance |
|---|---|---|---|
| A1 | M0 | Pure domain/state reducer | All legal/illegal transition fixtures; stop race idempotent |
| A2 | A1 | Capture/DSP component | Bounded callback, sample-rate fixtures, disconnect/overflow failure |
| A3 | A2 | Endpointing | Start timeout, pre/post-roll, short pause and cap fixtures |
| B1 | M0 | Shared worker protocol | Binary framing golden tests, limits and EOF handling |
| B2 | B1 | Worker inference | Pinned source, one resident model, bounded result |
| B3 | B2 | Cancellation supervisor | Kill/wait/reload, epoch fencing, no orphan process |
| B4 | B2 | Model manager | Hash/size validation, cancel/resume/import/atomic activation |
| C1 | A1, M0 | Target/permission adapters | Actual editable/secure/unknown classification with evidence |
| C2 | C1 | Delivery coordinator | Commit ledger, modifier release, single dispatch, no forced focus |
| C3 | C2, B3 | Full vertical slice | Inference result delivers once; cancel/focus-change retain safe outcome |
| D1 | A1 | Readiness/config IPC | Revision conflicts, migration, command permission negative tests |
| D2 | D1, B4 | Onboarding/settings | All setup and denial flows, real progress |
| D3 | C3, D1 | HUD/review | No focus stealing, scoped events, memory-only draft |
| E1 | C3, D2, D3 | Regression/bench suite | Corpus, controlled targets and release metrics |
| E2 | E1 | Signed package/docs | Clean offline-capable import test, upgrade/rollback evidence |

## 5. Engineering conventions

Rust core uses typed errors and explicit ownership; OS handles/FFI use narrow RAII wrappers. No unwrap/panic on user/device/model inputs. Callback-safe code is reviewed separately from ordinary asynchronous I/O. Domain tests should not require a webview. Generate TypeScript contract types from Rust where feasible and ensure generation is deterministic.

Prefer a small explicit state store over a large frontend orchestration framework. Frontend state renders coordinator snapshots; it does not determine whether a paste is authorized. No database is necessary for v1 configuration and a single memory-only draft.

CI on pull requests: Rust fmt/clippy/tests; strict TypeScript/lint/UI tests; worker/protocol build; lockfile and license checks. Native tests run on actual OS runners/hardware. macOS global permissions and third-party UI behaviors require a documented manual test path; headless UI tests cannot substitute for those.

## 6. Scope cuts if schedule is constrained

Keep: local CPU inference, correct endpointing, bounded memory, explicit stop/cancel, safe focus fallback, offline import, honest status and privacy behavior. Defer: Metal, clipboard preservation, push-to-talk, autostart convenience, secondary architectures, Linux automatic paste and advanced visual polish. A release may use tray-only status when a non-activating HUD is not proven. Never cut cancellation or target-validation safeguards to meet a date.
