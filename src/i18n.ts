// Minimal message catalog (docs/02 §6): English is complete; German and
// Indonesian catalogs exist as keys and fall back to English until reviewed
// translations are added. Transcription language is independent of UI locale.

const en = {
  "app.title": "Local Dictation",
  "onb.welcome.title": "Dictate into your apps",
  "onb.welcome.body": "Speech is processed on this computer. Audio and text are never sent to a server.",
  "onb.welcome.clipboard":
    "Pasting works through the clipboard: dictated text stays on the system clipboard afterwards, where clipboard history, sync and other apps can read it.",
  "onb.mic.title": "Microphone",
  "onb.mic.body": "Local Dictation listens only after you press the dictation shortcut, and stops when you finish.",
  "onb.mic.allow": "Allow microphone",
  "onb.mic.denied": "Microphone access is disabled.",
  "onb.mic.restricted": "Microphone access is blocked by a system policy on this Mac.",
  "onb.mic.test": "Test microphone",
  "onb.mic.stopTest": "Stop test",
  "onb.model.title": "Speech model",
  "onb.model.body": "Choose a model to download, or import a file you already have. Base is the balanced choice.",
  "onb.model.needed": "Install a model to continue.",
  "onb.shortcut.title": "Dictation shortcut",
  "onb.shortcut.body": "Press this shortcut in any app to start dictating. Press it again, or pause, to finish.",
  "onb.paste.title": "Automatic paste",
  "onb.paste.body":
    "To paste into the app you were using, macOS needs to let Local Dictation check which field is focused and send one paste command (Accessibility).",
  "onb.paste.enable": "Enable automatic paste",
  "onb.paste.copyOnly": "Use copy only",
  "onb.paste.recheck": "Check again",
  "onb.paste.granted": "Automatic paste is enabled.",
  "onb.paste.pending": "Turn on Local Dictation in System Settings → Privacy & Security → Accessibility, then check again.",
  "onb.paste.unavailable": "Automatic paste is not available on this system yet. Your text will be ready to copy.",
  "onb.trial.title": "Try it",
  "onb.trial.body":
    "Start a trial and say a short sentence. This checks the microphone and the local model. It does not test every app you will paste into.",
  "onb.trial.start": "Start trial",
  "onb.trial.finish": "Finish",
  "onb.trial.result": "Recognized text",
  "onb.done.title": "You're ready",
  "onb.done.body": "Local Dictation lives in the menu bar. Use the shortcut from any app.",
  "onb.done.launch": "Open Local Dictation when I log in",
  "nav.back": "Back",
  "nav.continue": "Continue",
  "nav.finish": "Done",
  "settings.openSettings": "Open System Settings",
  // Platform variants, picked by `tp` when they exist.
  "onb.paste.body.windows":
    "Local Dictation pastes into the field you started in. It checks that field with Windows UI Automation and sends one Ctrl+V. It never pastes into password fields, terminals or apps running as administrator.",
  "onb.mic.denied.windows":
    "Microphone access for desktop apps is turned off in Windows Settings → Privacy & security → Microphone.",
  "onb.mic.restricted.windows": "Microphone access is blocked by a policy on this PC.",
  "onb.done.body.windows": "Local Dictation lives in the notification area of the taskbar. Use the shortcut from any app.",
  "onb.done.body.linux": "Local Dictation lives in the system tray if your desktop shows one; you can also start dictating from this window.",
  "settings.openSettings.windows": "Open Windows Settings",
} as const;

export type MessageKey = keyof typeof en;
type Catalog = Partial<Record<MessageKey, string>>;

const catalogs: Record<string, Catalog> = { en, de: {}, id: {} };

let locale = "en";

export function setLocale(l: string): void {
  locale = l in catalogs ? l : "en";
}

export function t(key: MessageKey): string {
  return catalogs[locale]?.[key] ?? en[key];
}

/** `key` with a platform variant (`<key>.windows`, `<key>.linux`) when one exists. */
export function tp(key: MessageKey, platform: string): string {
  const variant = `${key}.${platform}`;
  return variant in en ? t(variant as MessageKey) : t(key);
}
