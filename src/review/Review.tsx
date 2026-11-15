import { useCallback, useState } from "react";
import { DraftEditor } from "../components/DraftEditor";
import { onOutcome, type Outcome } from "../ipc";
import { outcomeText } from "../messages";
import { useEvent } from "../useEvent";

const isDraftPending = (o: Outcome) => typeof o === "object" && "rejected" in o && o.rejected === "DRAFT_PENDING";

/** Review window: its capability allows only draft read/edit/copy/discard. */
export function Review() {
  const [seq, setSeq] = useState(0);
  const [note, setNote] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  useEvent(
    useCallback(
      () =>
        onOutcome((o) => {
          // The shell opens this window when the dictation shortcut was
          // pressed while a draft is waiting (docs/02 §4).
          const p = isDraftPending(o);
          setPending(p);
          setNote(p ? "" : outcomeText(o));
          setSeq((n) => n + 1);
        }),
      [],
    ),
  );
  const onError = useCallback((m: string) => { setError(m); }, []);
  const onEmpty = useCallback(() => { setPending(false); }, []);
  return (
    <main>
      <h1>Review text</h1>
      {pending && (
        <p role="alert" className="notice">
          A new dictation can't start while this text is waiting. Copy or discard it, and the new dictation starts.
          Close this window to keep the text without starting.
        </p>
      )}
      <p className="hint">Automatic paste is turned off while this window is open. Copy the text, or focus the destination and use your paste shortcut.</p>
      {error && <p role="alert" className="error">{error}</p>}
      <DraftEditor refreshKey={seq} onError={onError} onEmpty={onEmpty} />
      <p role="status" className="hint">{note}</p>
    </main>
  );
}
