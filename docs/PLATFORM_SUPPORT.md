# Platform support

What each platform supports, and what that is based on. "CI evidence" means a GitHub Actions run on a hosted runner, not a release qualification on reference hardware (docs/08 §5). Exact results are in `docs/IMPLEMENTATION_STATUS.md`.

| Platform | Tier | Capture + transcription | Global shortcut | Automatic paste | Copy | Sleep/lock clears drafts |
|---|---|---|---|---|---|---|
| macOS 13+ Apple Silicon | First release (pending GUI qualification) | Yes | Yes | Yes, with Accessibility, in qualified apps | Yes | Yes (NSWorkspace) |
| Windows 10/11 x64 (x86-64-v3 CPU) | Second release (pending qualification) | Yes | Yes | Yes, for same-integrity Edit/Document fields | Yes | Yes (WTS + power) |
| Linux X11 | Experimental | Yes | Yes | **No** (draft + Copy) | Yes | Yes (logind) |
| Linux Wayland | Manual workflow | Yes | **Unreliable**: use Start in the app window | **No** | Yes, with XWayland | Yes (logind) |

## Windows

**Requirements**
- Windows 10 or 11, x64.
- A CPU with AVX2/FMA (Intel Haswell 2013+ or AMD Excavator/Zen). Older CPUs are not supported; see ADR-012.
- WebView2. Windows 11 and current Windows 10 builds already have it. Otherwise the installer's embedded bootstrapper downloads it once, so offline machines need WebView2 installed first.

**Install**
- Run `Local Dictation_<version>_x64-setup.exe`. It installs for the current user under `%LOCALAPPDATA%\Local Dictation` and needs no administrator rights.
- Settings and models are stored in `%APPDATA%\app.localdictation.m0`.
- The installer is not signed yet, so SmartScreen shows a warning.

**Automatic paste** works when all of these hold:
- the foreground window and the focused element belong to the same process;
- the element is an enabled, writable UI Automation Edit or Document control;
- the target runs at the app's integrity level or lower.

**It never pastes into:**
- password fields;
- apps running as administrator (Windows would silently drop the keys);
- UAC prompts, the lock screen or other secure-desktop windows;
- terminals: Windows Terminal, cmd, PowerShell, conhost, PuTTY and other known terminal programs, matched by executable name.

**Microphone**
- Settings → Privacy & security → Microphone → *Let desktop apps access your microphone* must be on.
- Windows shows no prompt for desktop apps. The app reads the setting and links to the page.

**Supported-app matrix (docs/05 §8): not yet qualified.** The candidates are Notepad, a browser textarea, Word compose and VS Code. The CI native test checks a standard Win32 Edit control: Unicode paste is received exactly once, a field switch inside the window is detected, and a password field is Secure.

## Linux

See the Linux section, added with milestone M7.
