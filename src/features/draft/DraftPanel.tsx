import { useEffect, useState } from "react";
import { copyDraft, discardDraft, editDraft, getDraft, type DraftView, type Phase } from "../../ipc";
import { draftReasonText } from "../../messages";

interface Props {
  phase: Phase | null;
  /** Bumps whenever the coordinator reports a new outcome. */
  outcomeSeq: number;
  onError: (msg: string) => void;
}

/** Memory-only draft review (docs/02 §4). Edits create a new revision. */
export function DraftPanel({ phase, outcomeSeq, onError }: Props) {
  const [draft, setDraft] = useState<DraftView | null>(null);
  const [text, setText] = useState("");

  useEffect(() => {
    if (phase !== "draftReady") return;
    getDraft()
      .then((d) => {
        setDraft(d);
        setText(d?.text ?? "");
      })
      .catch((e: unknown) => { onError(String(e)); });
  }, [phase, outcomeSeq, onError]);

  // A stale draft object is never shown once the coordinator left DraftReady.
  if (!draft || phase !== "draftReady") return null;
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
    <section aria-labelledby="draft-h" className="draft">
      <h2 id="draft-h">Your text</h2>
      <p className="hint">{draftReasonText(draft.reason)} Kept in memory only, for up to 10 minutes.</p>
      <label htmlFor="draft-text" className="visually-hidden">Recognized text</label>
      <textarea id="draft-text" rows={5} value={text} onChange={(e) => { setText(e.target.value); }} />
      <div className="row">
        <button
          type="button"
          onClick={() => {
            save()
              .then((rev) => copyDraft(draft.id, rev))
              .catch((e: unknown) => { onError(String(e)); });
          }}
        >
          Copy
        </button>
        <button type="button" onClick={() => { discardDraft(draft.id).catch((e: unknown) => { onError(String(e)); }); }}>
          Discard
        </button>
      </div>
      <p className="hint">Copying leaves the text on the system clipboard, where other apps and clipboard history can read it.</p>
    </section>
  );
}
