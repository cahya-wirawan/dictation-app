import { useEffect, useState } from "react";
import { copyDraft, discardDraft, editDraft, getDraft, type DraftView } from "../ipc";
import { draftReasonText } from "../messages";

interface Props {
  /** Bumps whenever the draft may have changed (phase or outcome event). */
  refreshKey: number;
  onError: (msg: string) => void;
  onEmpty?: () => void;
}

function remaining(ms: number): string {
  const m = Math.floor(ms / 60_000);
  const s = Math.floor((ms % 60_000) / 1000);
  return `${String(m)}:${String(s).padStart(2, "0")}`;
}

/** Memory-only draft: edit (new revision), Copy, Discard, retention countdown. */
export function DraftEditor({ refreshKey, onError, onEmpty }: Props) {
  const [draft, setDraft] = useState<DraftView | null>(null);
  const [text, setText] = useState("");
  const [left, setLeft] = useState(0);

  useEffect(() => {
    getDraft()
      .then((d) => {
        setDraft(d);
        setText(d?.text ?? "");
        setLeft(d?.expiresInMs ?? 0);
        if (!d) onEmpty?.();
      })
      .catch((e: unknown) => { onError(String(e)); });
  }, [refreshKey, onError, onEmpty]);

  useEffect(() => {
    if (!draft) return;
    const started = Date.now();
    const initial = draft.expiresInMs;
    const timer = setInterval(() => { setLeft(Math.max(0, initial - (Date.now() - started))); }, 1000);
    return () => { clearInterval(timer); };
  }, [draft]);

  if (!draft) return <p className="hint">No text waiting.</p>;
  const dirty = text !== draft.text;

  const save = async (): Promise<number> => {
    if (!dirty) return draft.revision;
    await editDraft(draft.id, draft.revision, text);
    const fresh = await getDraft();
    if (fresh) {
      setDraft(fresh);
      setText(fresh.text);
    }
    return fresh?.revision ?? draft.revision;
  };

  return (
    <div className="draft">
      <p className="hint">{draftReasonText(draft.reason)}</p>
      <label htmlFor="draft-text">Recognized text</label>
      <textarea id="draft-text" rows={6} value={text} onChange={(e) => { setText(e.target.value); }} />
      <div className="row">
        <button
          type="button"
          onClick={() => {
            save().then((rev) => copyDraft(draft.id, rev)).catch((e: unknown) => { onError(String(e)); });
          }}
        >
          Copy
        </button>
        <button type="button" onClick={() => { discardDraft(draft.id).catch((e: unknown) => { onError(String(e)); }); }}>
          Discard
        </button>
      </div>
      <p className="hint">
        Kept in memory only. Cleared in <span aria-live="off">{remaining(left)}</span>, when you quit, or when the screen
        locks. Copying puts the text on the system clipboard, where clipboard history and other apps can read it.
      </p>
    </div>
  );
}
