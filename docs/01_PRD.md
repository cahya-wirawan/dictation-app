# 01 — Product requirements

## 1. Problem and outcome

Typing interrupts thought and can be slow or uncomfortable. Users need a lightweight desktop utility that turns a short spoken phrase into editable text in the application they are already using. Processing must run on the user's machine without sending recordings to a service.

Primary outcome: successful short dictation with minimal UI interaction. A successful automatic delivery means the intended text is observed in a controlled target during tests; in production the app can usually confirm only that paste input was dispatched. Product messages must respect that distinction.

## 2. Users and jobs

| User | Job | Product implication |
|---|---|---|
| Writer or office worker | Dictate paragraphs into mail, notes, and documents | Preserve focus; reliable Unicode and punctuation |
| Developer | Dictate comments and issue text | No shell execution, no automatic Enter, explicit newlines only |
| Privacy-conscious user | Work without uploading voice | Fully local capture and inference; offline import |
| Multilingual user | Dictate English, German, or Indonesian | Multilingual model and explicit language selection |
| User with accessibility needs | Avoid complex mouse interaction | Keyboard setup, visible state, configurable shortcut |

## 3. Scope and release tiers

P0 is required for the first macOS release. P1 adds Windows and conveniences after P0 gates. P2 is exploratory. A supported platform must meet its own gates; compilation alone is not support.

| ID | Priority | Requirement and acceptance |
|---|---|---|
| FR-01 | P0 | Tray/menu-bar resident app; closing settings leaves the app running; explicit Quit stops capture and worker. |
| FR-02 | P0 | Configurable global toggle shortcut. First key-down begins; second distinct key-down stops. Auto-repeat and key-up do not toggle. A conflicting shortcut leaves the old valid binding active and shows an error. |
| FR-03 | P0 | Microphone selection and level test. Capture uses the chosen device only. If unavailable, show a device error and ask for reselection; never silently switch a live recording. |
| FR-04 | P0 | Native audio capture with visible listening indicator. Capture begins only after a deliberate start action; no idle microphone stream. |
| FR-05 | P0 | End after sustained post-speech silence; configurable 500–2500 ms, default 1000 ms. Explicit stop bypasses this timer. No speech within 8 s ends with “No speech detected” and no transcription or clipboard write. |
| FR-06 | P0 | Entire inference runs locally with no cloud fallback. Installed model works with networking disabled. |
| FR-07 | P0 | Supported settings: automatic language detection, English, German, Indonesian. Use transcription, never translation; no LLM rewriting. |
| FR-08 | P0 | Default automatic paste on validated supported targets. Capture target before showing UI. A changed/unknown target yields an in-memory draft without auto-copy or paste. |
| FR-09 | P0 | Explicit Copy and “Paste to focused app” recovery actions; the latter is a separately configured global shortcut. No automatic retry following a dispatched paste. |
| FR-10 | P0 | Global cancel shortcut active during a session, plus tray Cancel. Cancellation invalidates all outstanding session callbacks and forbids subsequent text delivery. |
| FR-11 | P0 | Download approved model with progress/cancel/resume or import approved local model offline. Verify bytes and SHA-256 against a bundled manifest before activation. |
| FR-12 | P0 | Errors preserve a completed text draft in memory when available; audio and text never appear in logs by default. |
| FR-13 | P0 | Settings persist atomically and migrate by schema version. No persistent audio or transcript history. |
| FR-14 | P0 | New capture while busy is rejected with status; no hidden queue and no simultaneous sessions. |
| FR-15 | P0 | Session maximum 120 s. At the cap, finish and transcribe, with a visible “Maximum recording length reached” notice. |
| FR-16 | P0 | If a draft exists, new recording does not overwrite it. A Settings/Review action must copy or discard the draft first. |
| FR-17 | P1 | Windows x64 release with native capture, shortcut, Unicode clipboard and ordinary target paste, independently tested. |
| FR-18 | P1 | Optional push-to-talk with explicit key-release and lost-release behavior. Not part of v1 toggle semantics. |
| FR-19 | P2 | Linux X11 tested adapter; compositor-specific Wayland portal experiments. Never advertise universal Linux auto-paste. |

## 4. Core workflow

1. User focuses an editable field and presses their dictation shortcut.
2. Rust checks readiness and captures a target token; permission dialogs belong in onboarding, not the recording path.
3. App starts capture and displays a non-activating indicator. Speech onset is detected after a short sustained voiced interval.
4. User speaks. Silence timer resets whenever speech resumes. A second shortcut press can finish immediately.
5. App releases the microphone, finalizes audio, transcribes locally, and checks the transcript for empty or rejected output.
6. Delivery validates permissions, focus, target exclusions, and released shortcut modifiers. If safe, stage plain text and send exactly one paste chord.
7. Show “Paste sent” or retain a draft with a specific explanation. Never imply the app read back the receiving document.

## 5. Defaults

| Setting | Default | Allowed behavior |
|---|---|---|
| Trigger | Toggle | One registered configurable chord; onboarding proposes Ctrl+Alt+Space and checks actual registration |
| Cancel | Ctrl+Alt+Escape candidate | Active only while busy; handle conflict explicitly; tray Cancel always exists |
| Paste draft | Ctrl+Alt+V candidate | Explicit opt-in binding, useful after focus-change fallback |
| Language | Auto | Explicit language preferred for very short phrases |
| Model | Base multilingual candidate | Optional Small multilingual; CPU baseline mandatory |
| Silence | 1000 ms | 500–2500 ms in 100 ms steps |
| No speech timeout | 8000 ms | Advanced config 3000–15000 ms |
| Maximum capture | 120000 ms | Fixed v1 resource cap |
| Pre-roll | 300 ms | Only within an explicitly active recording |
| Post-roll | 200 ms | Included from already recorded silence; never wait twice |
| Model residency | Keep while app runs | Explicit Unload action; no inference/capture in idle |
| Draft retention | Memory only | 10 minute TTL; explicit clear; cleared on quit/lock |
| Clipboard | Leave dictation text | Clearly disclosed; optional preservation mode described separately |
| Launch at login | Off | User opt-in only |

Shortcut candidates are product defaults, not guaranteed free OS combinations. Keyboard layout and system reservations must be tested.

## 6. Nonfunctional requirements

- Privacy: zero audio/transcript network traffic; no listener on TCP/UDP ports. Model/update traffic is separate and user initiated in v1.
- Latency: warm start-to-first-captured-sample p95 ≤300 ms; end-of-speech-to-stop uses configured silence threshold plus ≤200 ms scheduling overhead; 10 s phrase stop-to-paste-dispatch p95 ≤3 s on the designated reference Mac using the selected base profile. These are release targets, not universal promises.
- Reliability: no duplicate dispatch in deterministic and fault-injection tests; no dispatch after cancellation or target mismatch.
- Responsiveness: inference and audio processing never block UI/main thread. Settings remain responsive during inference.
- Resources: 120 s mono float32 audio is 7.68 MB at 16 kHz. Explicit budgets must also include native-rate ring buffer, resampler, model, tensors and webview. Reference base-profile total RSS target ≤1 GB; idle CPU mean <1% of one logical core over 60 s with UI closed.
- Accessibility: keyboard-operable screens, accessible names, sufficient contrast, reduced motion, and non-color status cues. Status announcements must not spam assistive technology with meter updates.
- Offline: clean import plus inference must work without any downloader. Installation prerequisites and OS webview dependencies are documented by platform.

## 7. Non-goals

Always-on wake words, meeting recording, speaker diarization, translation, remote inference, LLM cleanup, command execution, clinical accuracy certification, password-field dictation, guaranteed insertion into every app, and persistent transcript history are outside v1.

## 8. Acceptance scenarios

- Given an installed model and approved permissions, dictating “Hallo, das ist ein Test” into TextEdit inserts once and leaves focus in TextEdit.
- Given no speech, eight seconds elapse and recording closes without clipboard mutation.
- Given a pause shorter than threshold, the same session continues; after threshold, it finishes exactly once.
- Given focus changes during inference, the transcript becomes a draft and neither app receives automatic text.
- Given user cancel during inference, a late worker result is discarded and no paste occurs.
- Given input-denied permission, the app can still offer Copy after transcription when capture was permitted.
- Given a corrupt model download, the model is never loaded and the last valid model remains intact.

## 9. Product metrics and boundaries

Measure locally during opt-in testing: session completion, fallback reasons, end-to-end latency, WER/CER, and controlled-target insertion success. Production analytics are absent in v1. Logs cannot prove correct insertion into arbitrary third-party apps. Ship only with an explicit supported-app list and transparent limitations.
