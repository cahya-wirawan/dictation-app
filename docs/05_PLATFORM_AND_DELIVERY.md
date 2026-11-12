# 05 — Platform permissions and text delivery

## 1. Support policy

| Platform | Release tier | Capture/trigger | Automatic delivery |
|---|---|---|---|
| macOS Apple Silicon | First release | CPAL/CoreAudio; official shortcut plugin candidate | Native target probe, paste chord; Accessibility permission and tested non-activating HUD |
| macOS Intel | Later qualification | Same design; separate binary/build validation | Only after measured compatibility; no implied support from universal build |
| Windows x64 | Second release | CPAL/WASAPI; official shortcut plugin candidate | Native foreground HWND check and SendInput; ordinary integrity targets only |
| Linux X11 | Experimental | CPAL supported host; X11 shortcut adapter | X11 target/clipboard/input backend on named tested desktops |
| Linux Wayland | Manual workflow initially | In-app recording; global shortcut only after actual portal support test | Manual Copy; automatic paste remains disabled until compositor-specific validation |

Choose minimum supported OS versions in M0 based on Tauri/system-webview prerequisites and tested machines. Do not infer support from a browser's reported OS string. Installer documentation must publish exact tested OS/build/architecture values before release.

Tauri's global-shortcut plugin exposes desktop shortcut registration [S2]. That does not establish that every Linux compositor supports the required global behavior. Wayland GlobalShortcuts and RemoteDesktop are separate portal interfaces [S9, S10]; a shortcut portal does not itself grant input injection, clipboard access, or reliable target identity. Runtime-probe each capability and degrade explicitly.

## 2. macOS permissions

- Include a clear `NSMicrophoneUsageDescription` in Info.plist [S11]. Use a direct native microphone authorization flow where needed; permission request must occur on user action.
- Configure microphone/audio-input entitlement as appropriate to the actual signing/hardened-runtime/sandbox configuration; verify the signed release artifact on a clean machine. Do not assume debug behavior represents distribution behavior.
- Check Accessibility trust through Apple's APIs and prompt only in the explicit “Enable automatic paste” flow [S8]. A System Settings link plus recheck is required; the grant is not immediate merely because a dialog opened.
- Global shortcut registration is not a request for broad keyboard surveillance. Do not add a general keylogger. Assess Input Monitoring only if the chosen implementation actually needs it; avoid requesting unnecessary access.
- Do not request Screen Recording or Apple Events Automation simply to paste. Native frontmost-app and Accessibility target checks should be investigated first.
- Use a stable bundle identifier and properly signed/notarized build so permissions and app identity behave predictably across updates [S12]. Never advise users to disable platform security as the installation path.

## 3. Windows permissions and limitations

Use native desktop capture and document OS microphone privacy controls, including denial for desktop applications. Probe the selected device instead of treating device enumeration as permission success. Do not request administrator privileges for normal use.

GetForegroundWindow and native process/window identity form the target token. Keyboard input uses a tested SendInput implementation; the API is constrained by User Interface Privilege Isolation, so it cannot reliably inject into higher-integrity targets [S7]. Elevated programs, UAC/secure desktop, lock screen, and secure credential fields are unsupported. A failed send must not cause automatic escalation or indefinite retries.

Support clipboard Unicode text explicitly. Test WebView2 installation and offline installer behavior; a Tauri executable alone does not remove the system-webview dependency.

## 4. Target token and validation

Capture the target immediately on shortcut start, before any dictation UI interaction. Token includes platform session/desktop identity, process identity resistant to PID reuse, native window identity, and focused editable element identity where available. Hold native Accessibility references safely; serialize no raw pointers to the frontend.

Validate again after inference and immediately before input dispatch:

1. Session is live, has usable text, is not canceled and has no committed delivery.
2. Target is not this app, OS shell/login screen, excluded app, or known protected/secure field.
3. Current foreground process/window matches the start token.
4. If a focused-element token was available, it still matches and is editable/unprotected. Missing/invalid element information is an unknown capability, not evidence of a safe text field.
5. Shortcut modifiers have been released within 1500 ms; otherwise retain a draft without sending keys.
6. Native platform can observe adequate target identity. If not, automatic delivery is disabled for that environment.

If same-window focus moved to another element, skip auto-paste when that can be detected. Caret position changes inside an unchanged field cannot generally be tracked safely without intrusive monitoring. Explain this residual limitation: users should keep the destination focused until completion. A final check cannot make the OS send atomic with focus; test the race, minimize the interval, and never claim a universal guarantee.

For targets whose editable-element identity is unavailable, the first release uses explicit draft delivery, even if a weaker window-only heuristic would be convenient. Add tested app-specific adapters later through an ADR.

## 5. Clipboard transaction: default mode

Default `leave_text` is intentionally simple and disclosed: the recognized phrase remains on the system clipboard after a successful automatic paste attempt or explicit Copy. The app does not claim clipboard privacy or full restoration.

Algorithm:

1. Finish inference and create an in-memory transcript associated with session/draft revision.
2. Run target and permission validation. If blocked, retain draft; do not mutate clipboard.
3. Wait for physical trigger modifiers to be released. Never synthesize releases for physical keys the user is holding.
4. Acquire an application-local delivery lock; OS clipboard locking/retry must be bounded to ≤500 ms. Clipboard write retries are allowed only before input dispatch.
5. Write plain Unicode text. Do not append Return, execute transcript commands, or write HTML.
6. Recheck target. If changed after the write, skip input and retain draft. The clipboard may now contain dictated text; report this accurately. Clipboard mutation cannot be made atomic with focus.
7. Mark dispatch committed and send one complete paste chord (Cmd+V on macOS, Ctrl+V on ordinary Windows editors; target-specific terminal bindings require explicit profile).
8. Always release only synthetic keys issued by this transaction, including on API error. Record sent/uncertain based on actual API return. Never infer document acceptance from SendInput success.
9. Clear active audio/text from app memory after a sent outcome; show “Paste sent.” On blocked/uncertain result retain a draft with outcome metadata and require explicit action for resend.

No automatic Enter. Multiline paste can still have application-specific effects, especially in terminals; terminal targets are excluded from automatic delivery by default. User may enable a named, tested terminal profile after a separate confirmation about pasted commands/newlines. Exclusions are evaluated by native app identity, not a window title alone.

## 6. Optional clipboard preservation

This is P1 and must not block first release. A generic text-only clipboard API cannot restore every native format or promise that the receiving app has consumed data. Tauri's clipboard plugin supplies clipboard operations [S3], not proof of completed paste.

Offer preservation only on adapters that can snapshot all advertised formats and track ownership/revision reliably. Otherwise label it unavailable; do not silently replace an image/rich clipboard with a text-only backup. Bound snapshots (candidate 8 MiB), reject delayed/promised/unsupported data, keep only in memory, and never log contents.

After dispatch, keep clipboard text available for a configurable conservative grace interval (candidate 1500 ms). Restore only if the OS clipboard still has the app's exact ownership/revision token. A text-equality check alone is insufficient. If another app/user changed clipboard ownership, leave the new content alone. Even a grace interval is heuristic because arbitrary apps expose no paste-consumed acknowledgment; preserve mode is best-effort and must say so. Clipboard history, managers and OS sync can retain dictated text despite restoration.

## 7. Copy-only and explicit draft paste

Copy-only sessions create a draft without writing to clipboard. User selects Copy to stage text, then pastes normally. This preserves deliberate clipboard control.

For explicit paste shortcut: user focuses the intended app, invokes paste-draft chord, and Rust captures a new target token at that moment. Validate editable/protected status, draft revision and permissions, release modifiers and run one transaction. This authorizes the newly focused target rather than silently refocusing the original app. If target inspection is unavailable, offer Copy; never force focus to a stale app.

## 8. Qualification matrix

For each target app/version test plain text, selections, multiline, umlauts, emoji, CJK, empty input, text field changes, window switches, secure fields, layout variations and permissions revoked mid-session. First Mac candidates: TextEdit, Notes, browser textarea, mail compose, VS Code editor. First Windows candidates: Notepad, browser textarea, Word compose and VS Code. An app enters the supported list only after the target-element adapter and delivery tests pass.
