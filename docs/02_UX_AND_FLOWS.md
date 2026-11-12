# 02 — UX and interaction specification

## 1. Interface surfaces

The normal interaction has no main window. A menu-bar/tray icon exposes status, Start/Stop, Cancel, Review draft, Settings, and Quit. Recording uses a compact, non-activating HUD near the lower center of the active display, avoiding common text-caret areas. Position can be adjusted in settings. Do not assume Tauri's always-on-top flag alone creates a truly non-activating panel; prove the native adapter during the first spike.

| Surface | Content | Focus policy |
|---|---|---|
| Tray menu | Readiness, current mode, commands | Opening it may change focus; active session target is then revalidated |
| Listening HUD | Mic icon, “Listening”, level bars, elapsed time, stop hint | Native non-activating, no keyboard focus |
| Transcribing HUD | “Transcribing locally…”, elapsed processing time, cancel hint | No focus and no guessed percentage |
| Draft banner | “Text ready — target changed. Review or paste with your shortcut.” | No automatic editor opening |
| Review window | Editable draft, Copy, Discard, retention countdown | Deliberately takes focus; automatic delivery is disabled |
| Settings | Devices, shortcut, models, language, silence, permissions, privacy | Ordinary accessible window |

If a reliable HUD cannot be implemented on a platform, use tray-only status rather than a focus-stealing replacement. HUD transport exposes no transcript or target metadata.

## 2. Onboarding

1. Welcome: “Dictate into your apps. Speech is processed on this computer.” State that copied/pasted text is exposed to the OS clipboard and potentially clipboard history/sync.
2. Microphone: explain why access is needed, request on explicit Continue, show device list and Test microphone. Testing opens capture only while this page's test is active and closes on navigation.
3. Model: show multilingual Base and Small candidates, manifest-derived download size, installed disk usage estimate, CPU/Metal label, and import option. Never invent fixed memory or latency figures before measurements exist.
4. Shortcut: editable chord capture field and OS registration check. Failed registration must not advance as if successful.
5. Automatic paste: explain Accessibility on macOS or platform limitations; offer “Enable automatic paste” or “Use copy only.” Recheck after returning from OS settings.
6. Trial: a local onboarding text field makes one supervised capture/transcribe/deliver cycle. Explain that this verifies setup, not all destination applications.
7. Done: show the selected chord and the menu-bar location. Keep autostart off unless selected.

Readiness requires a valid device, capture permission, verified model, worker health, and trigger binding. Paste permission is separate: lack of it allows a copy-only workflow with explicit Copy.

## 3. Settings structure

| Page | Controls |
|---|---|
| General | Launch at login, show HUD, reduced motion, tray icon behavior |
| Recording | Microphone, test meter, silence threshold, no-speech timeout, selected-device policy |
| Shortcuts | Dictation toggle, cancel, optional paste draft, conflict messages |
| Transcription | Model, language, CPU/Metal preference, unload model, inference status |
| Models | Installed/available entries, verified state, progress, cancel/import/delete |
| Delivery | Automatic paste or copy-only, app exclusions, clipboard policy, platform capability status |
| Privacy | Data lifecycle explanation, Clear draft, redacted diagnostics export |
| About | Version, engine revision, licenses, supported OS, manual update link |

Model deletion/switching is disabled while its worker job is active. Apply microphone/model changes after the session, never mid-recording. Threshold changes likewise affect the next session.

## 4. Interaction details

- Holding the toggle chord must not repeatedly start and stop. Require release before accepting the next toggle press.
- Cancel uses a scoped global chord and tray action. Escape by itself only cancels when a dictation-owned window has focus; globally capturing Escape would interfere with the receiving app.
- Opening Review intentionally opts out of pending automatic paste. Editing a draft creates a new draft revision; a stale delivery request must fail validation.
- Starting while a draft exists shows “Review, copy, or discard your previous text first.” It must not discard silently.
- Start/stop sounds are off by default; if enabled, stop the start cue before opening capture so it is not recorded. Do not use an inference-complete sound that hides delivery failure.
- Never append Enter/Return. Preserve transcript line breaks only; no automatic message send or terminal execution.
- Do not silently retry after a paste attempt: a receiving editor may already contain the text.

## 5. Status and error copy

| Condition | Copy | Recovery |
|---|---|---|
| Model not installed | “Install or import a local model to begin.” | Open Models |
| Microphone denied | “Microphone access is disabled.” | Open platform privacy settings; recheck |
| Selected mic removed | “Your selected microphone disconnected. Recording stopped.” | Discard incomplete audio; select device |
| Initial silence | “No speech detected.” | Return ready; no draft |
| Low-quality/empty result | “No usable text was recognized. Try speaking closer to the microphone.” | Clear audio; retry |
| Target changed | “Text ready. The focused app changed, so automatic paste was skipped.” | Review / explicit paste shortcut |
| Clipboard unavailable | “Text ready. The clipboard is busy.” | Copy retry, no automatic paste retry |
| Input dispatch blocked | “Automatic paste is unavailable here. Your text is ready to copy.” | Review/Copy |
| Input sent | “Paste sent.” | No automatic resend |
| Worker timeout | “Local transcription took too long.” | Clear audio; restart worker; retry recording |
| Clipboard restore skipped | “Clipboard changed; your newer clipboard content was kept.” | Informational only |

No error notification contains the spoken text. Permission denial is a supported state, not a fatal crash. Notifications with sensitive text are prohibited.

## 6. Accessibility and localization

Use semantic controls and visible labels; meter bars are decorative with one stable accessible label. Announce phase transitions, never every audio frame. Use at least 4.5:1 text contrast, visible focus rings, 200% text zoom and keyboard navigation. Support VoiceOver first, then Narrator for Windows. UI starts in English with localization keys for German and Indonesian; transcription language is independent of UI locale. Avoid transcript normalization that strips umlauts or changes Indonesian names.
