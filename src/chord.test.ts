import { describe, expect, it } from "vitest";
import { chordFromEvent, displayChord, type KeyLike } from "./chord";

const k = (code: string, mods: Partial<KeyLike> = {}): KeyLike => ({
  code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods,
});

describe("chordFromEvent", () => {
  it("builds layout-independent chords from codes", () => {
    expect(chordFromEvent(k("Space", { ctrlKey: true, altKey: true }))).toEqual({ kind: "chord", chord: "Ctrl+Alt+Space" });
    expect(chordFromEvent(k("KeyV", { metaKey: true, shiftKey: true }))).toEqual({ kind: "chord", chord: "Shift+Cmd+KeyV" });
    expect(chordFromEvent(k("F5", { altKey: true }))).toEqual({ kind: "chord", chord: "Alt+F5" });
  });

  it("waits while only modifiers are held", () => {
    expect(chordFromEvent(k("ControlLeft", { ctrlKey: true }))).toEqual({ kind: "incomplete" });
    expect(chordFromEvent(k("MetaRight", { metaKey: true }))).toEqual({ kind: "incomplete" });
  });

  it("rejects chords that would capture ordinary typing", () => {
    expect(chordFromEvent(k("KeyA"))).toEqual({ kind: "invalid", reason: "needsModifier" });
    expect(chordFromEvent(k("KeyA", { shiftKey: true }))).toEqual({ kind: "invalid", reason: "shiftOnly" });
    expect(chordFromEvent(k("NumpadEnter", { ctrlKey: true }))).toEqual({ kind: "invalid", reason: "unsupportedKey" });
  });
});

describe("displayChord", () => {
  it("uses macOS symbols and plain text elsewhere", () => {
    expect(displayChord("Ctrl+Alt+Space", true)).toBe("⌃⌥Space");
    expect(displayChord("Shift+Cmd+KeyV", true)).toBe("⇧⌘V");
    expect(displayChord("Ctrl+Alt+Digit1", false)).toBe("Ctrl+Alt+1");
  });
});
