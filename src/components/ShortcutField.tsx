import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { chordFromEvent, displayChord } from "../chord";
import { pauseShortcuts, resumeShortcuts, setShortcut, type ShortcutSlot } from "../ipc";

interface Props {
  slot: ShortcutSlot;
  label: string;
  value: string | null;
  mac: boolean;
  optional?: boolean;
  onSaved: (chord: string | null) => void;
}

const reasonText = {
  needsModifier: "Include Control, Option, Command or Shift.",
  unsupportedKey: "That key can't be used in a shortcut.",
  shiftOnly: "Shift alone with a key would interfere with typing. Add Control, Option or Command.",
};

/**
 * Chord capture (docs/02 §2 step 4). While focused, the global shortcuts are
 * paused so pressing them here does not start dictation. A chord is saved only
 * after Rust registered it with the OS; failures are shown, never hidden.
 */
export function ShortcutField({ slot, label, value, mac, optional = false, onSaved }: Props) {
  const [pending, setPending] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const paused = useRef(false);

  useEffect(() => () => {
    if (paused.current) void resumeShortcuts();
  }, []);

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    // Plain Tab / Shift+Tab keep normal keyboard navigation.
    if (e.code === "Tab" && !e.ctrlKey && !e.altKey && !e.metaKey) return;
    e.preventDefault();
    const r = chordFromEvent(e);
    if (r.kind === "chord") {
      setPending(r.chord);
      setMessage("");
    } else if (r.kind === "invalid") {
      setMessage(reasonText[r.reason]);
    }
  };

  const save = (chord: string | null) => {
    setBusy(true);
    setShortcut(slot, chord)
      .then((res) => {
        setPending(null);
        setMessage(chord ? "Saved and registered." : "Removed.");
        onSaved(res.chord);
      })
      .catch((e: unknown) => {
        const s = String(e);
        setMessage(s.includes("SHORTCUT_CONFLICT") ? "Another app or system feature already uses this shortcut. Try a different one." : s);
      })
      .finally(() => { setBusy(false); });
  };

  const id = `shortcut-${slot}`;
  const shown = pending ?? value;
  return (
    <div className="shortcut">
      <label htmlFor={id}>{label}</label>
      <input
        id={id}
        readOnly
        value={shown ? displayChord(shown, mac) : "Not set"}
        aria-describedby={`${id}-hint`}
        onFocus={() => { paused.current = true; void pauseShortcuts(); }}
        onBlur={() => { paused.current = false; void resumeShortcuts(); }}
        onKeyDown={onKeyDown}
      />
      <div className="row">
        <button type="button" disabled={!pending || busy} onClick={() => { save(pending); }}>Save shortcut</button>
        {optional && value !== null && <button type="button" disabled={busy} onClick={() => { save(null); }}>Remove</button>}
      </div>
      <p id={`${id}-hint`} className="hint" role="status">{message || "Click the field, then press the keys you want to use."}</p>
    </div>
  );
}
