# 12 — Sources and evidence boundaries

Reviewed 12 November 2026. These references support platform/library capabilities. They do not demonstrate that this proposed app exists, that a chosen dependency matrix compiles, or that performance targets have been met. Pin exact upstream revisions when implementing; mutable `master` pages are discovery references only.

| ID | Official source | Supports |
|---|---|---|
| S1 | [whisper.cpp repository](https://github.com/ggml-org/whisper.cpp) and [stream example](https://github.com/ggml-org/whisper.cpp/tree/master/examples/stream) | Local native Whisper engine, backend/model options and VAD/stream examples; examples are not a production dictation app |
| S2 | [Tauri 2 Global Shortcut](https://v2.tauri.app/plugin/global-shortcut/) | Desktop plugin registration/event interface; platform support still needs testing |
| S3 | [Tauri 2 Clipboard](https://v2.tauri.app/plugin/clipboard/) | Clipboard operations; not a completed-paste acknowledgment or universal clipboard restoration |
| S4 | [Tauri capabilities](https://tauri.app/security/capabilities/) and [configuration reference](https://v2.tauri.app/reference/config/) | Window/webview permissions, capability composition and custom-command restriction considerations |
| S5 | [Tauri embedding external binaries](https://v2.tauri.app/develop/sidecar/) | Bundled native binary configuration, target naming and execution considerations |
| S6 | [CPAL source documentation](https://github.com/RustAudio/cpal/blob/master/src/lib.rs) and [repository](https://github.com/RustAudio/cpal) | Native audio capture APIs, callback behavior and supported host considerations |
| S7 | [Microsoft SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput) | Input API return semantics and UIPI integrity restrictions |
| S8 | [Apple AXIsProcessTrustedWithOptions](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions) | Accessibility trust check/prompt behavior |
| S9 | [XDG GlobalShortcuts portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html) | Session-based shortcut registration and activation/deactivation |
| S10 | [XDG RemoteDesktop portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html) | Separate remote-input session interface; actual compositor/backend availability must be established |
| S11 | [Apple NSMicrophoneUsageDescription](https://developer.apple.com/documentation/BundleResources/Information-Property-List/NSMicrophoneUsageDescription) | Microphone purpose-string property |
| S12 | [Tauri macOS code signing](https://v2.tauri.app/distribute/sign/macos/) | Signing and notarization workflow for distributed Mac applications |
| S13 | [Tauri distribution](https://v2.tauri.app/distribute/) | Platform-specific build and installer overview |

## Proposed design rather than sourced fact

The selected worker architecture, frame protocol, model candidates, silence/onset timings, TTLs, shortcut candidates, phased release order, resource budgets, WER targets and engineering estimates are recommendations for this product. They require implementation and measurement. No source is being cited as evidence of those numerical values.

## Further verification at M0

Open the actual pinned whisper C header and build configuration before writing FFI; validate a maintained VAD and resampler implementation; inspect platform target-element APIs and tested clipboard formats; establish minimum OS/webview requirements from the selected Tauri version. Do not rely on an unofficial mirror or stale tutorial for current library behavior.
