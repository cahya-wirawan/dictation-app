# Platform support

What each platform supports, and what that is based on. "CI evidence" means a GitHub Actions run on a hosted runner, not a release qualification on reference hardware (docs/08 §5). Exact results are in `docs/IMPLEMENTATION_STATUS.md`.

| Platform | Tier | Capture + transcription | Global shortcut | Automatic paste | Copy | Sleep/lock clears drafts |
|---|---|---|---|---|---|---|
| macOS 13+ Apple Silicon | First release (pending GUI qualification) | Yes | Yes | Yes, with Accessibility, in qualified apps | Yes | Yes (NSWorkspace) |
| macOS 13+ Intel (universal DMG) | Same build; not yet qualified on Intel hardware (ADR-013) | Yes (CPU, x86-64-v3) | Yes | Yes, with Accessibility, in qualified apps | Yes | Yes (NSWorkspace) |
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

## Linux (experimental, M7)

M7 was a time-boxed experiment (docs/07 §2). The result is a **limited** support statement, not a claim of broad desktop support.

**What works**
- Microphone capture through CPAL/ALSA (or PipeWire/PulseAudio through their ALSA plugins) and local transcription. This is the same code as on the other platforms.
- **Copy** on X11, and on Wayland sessions that run XWayland. The app owns the X11 `CLIPBOARD` selection until another app copies something. Most Wayland desktops bridge that selection to native Wayland apps.
- **Sleep, shutdown and lock** cancel the session and clear the draft, through systemd-logind (`PrepareForSleep`, `PrepareForShutdown`, `Session.Lock`).
- **Global shortcut on X11**, through the Tauri shortcut plugin's X11 key grab.

**What does not**
- **Automatic paste: never.** Neither X11 nor Wayland gives the app the identity of the focused text field without AT-SPI, which has not been qualified. Window identity alone (EWMH `_NET_ACTIVE_WINDOW`, `_NET_WM_PID`, `WM_CLASS`) is captured and checked, so a window switch is reported. But docs/05 §4 requires element identity before any automatic delivery. Dictated text always becomes a draft you copy.
- **Global shortcut on Wayland:** unreliable. The X11 grab only sees keys while an XWayland window is focused. The app detects Wayland and tells you to use **Start** in its window.
  - The xdg-desktop-portal *GlobalShortcuts* interface is probed and reported, not used. Using it needs a portal session per desktop, and that has not been tested.
- **Pure Wayland without XWayland** has no Copy, and the UI says so.
- **Packaging:** `.deb` only, for Ubuntu 24.04 (CI builds it, installs it with apt, runs the installed worker and removes it). AppImage, Flatpak and RPM were not attempted. A Flatpak would also need portal-based microphone and clipboard access.

**Session probe** (`dictation_platform::probe`, shown in the app on Linux):

| Field | Meaning |
|---|---|
| `kind` | `x11`, `wayland` or `none`, from `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY` and `DISPLAY` |
| `x11_reachable` | An X server (native or XWayland) accepted a connection, so Copy works |
| `ewmh_active_window` | A window manager publishes `_NET_ACTIVE_WINDOW`, so window changes are detected |
| `global_shortcuts_reliable` | Native X11 session with a reachable X server |
| `portal_global_shortcuts` | A *running* xdg-desktop-portal exports GlobalShortcuts. The probe never D-Bus-activates the portal. |
| `logind` | `org.freedesktop.login1` is on the system bus, so sleep and lock clear drafts |

**Tested environments** (no physical desktop was available; nothing below is a named-desktop qualification):

| Environment | Result |
|---|---|
| Ubuntu 22.04 host, tty session (no display) | Probe `kind: none`, no X11, logind present. Capture, transcription and benchmarks work. Copy is unavailable, and the UI says so. |
| Same host under Xvfb (no window manager) | Probe `kind: x11`, `ewmh_active_window: false`. The Unicode clipboard round trip and ownership handover pass, and so does the simulated-EWMH window-change test. |
| GitHub Ubuntu 24.04 runner, Xvfb | The same native tests in CI. The `.deb` installs with apt, and its worker transcribes the JFK fixture. |
| GNOME, KDE Plasma, Sway, Xfce on real hardware | **Not tested.** Required before any desktop is named as supported. |
