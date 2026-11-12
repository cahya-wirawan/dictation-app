// Keyboard event → global-shortcut chord ("Ctrl+Alt+Space"), using
// KeyboardEvent.code so the chord does not depend on the keyboard layout.
// Only used while the in-app capture field is focused; nothing is logged.

export interface KeyLike {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

const MODIFIER_CODES = new Set([
  "ControlLeft", "ControlRight", "AltLeft", "AltRight", "ShiftLeft", "ShiftRight",
  "MetaLeft", "MetaRight", "OSLeft", "OSRight", "CapsLock", "Fn",
]);

/** Keys allowed as the chord's main key. */
function keyName(code: string): string | null {
  if (/^Key[A-Z]$/.test(code) || /^Digit[0-9]$/.test(code) || /^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  const named = ["Space", "Escape", "Enter", "Tab", "Backspace", "Backquote", "Minus", "Equal",
    "BracketLeft", "BracketRight", "Backslash", "Semicolon", "Quote", "Comma", "Period", "Slash",
    "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"];
  return named.includes(code) ? code : null;
}

export type ChordResult =
  | { kind: "incomplete" }
  | { kind: "invalid"; reason: "needsModifier" | "unsupportedKey" | "shiftOnly" }
  | { kind: "chord"; chord: string };

export function chordFromEvent(e: KeyLike): ChordResult {
  if (MODIFIER_CODES.has(e.code)) return { kind: "incomplete" };
  const key = keyName(e.code);
  if (!key) return { kind: "invalid", reason: "unsupportedKey" };
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Cmd");
  if (mods.length === 0) return { kind: "invalid", reason: "needsModifier" };
  // Shift+letter would hijack ordinary typing of capitals.
  if (mods.length === 1 && mods[0] === "Shift") return { kind: "invalid", reason: "shiftOnly" };
  return { kind: "chord", chord: [...mods, key].join("+") };
}

const MAC_SYMBOLS: Record<string, string> = { Ctrl: "⌃", Alt: "⌥", Shift: "⇧", Cmd: "⌘" };

/** Human-readable chord, e.g. "⌃⌥Space" on macOS or "Ctrl+Alt+Space". */
export function displayChord(chord: string, mac: boolean): string {
  const parts = chord.split("+").map((p) => p.replace(/^Key/, "").replace(/^Digit/, ""));
  if (!mac) return parts.map((p) => (p.toLowerCase() === "alt" ? "Alt" : p)).join("+");
  const key = parts.pop() ?? "";
  return parts.map((p) => MAC_SYMBOLS[p] ?? p).join("") + key;
}
