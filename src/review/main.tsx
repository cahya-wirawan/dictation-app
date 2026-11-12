import { StrictMode, useCallback, useState } from "react";
import { createRoot } from "react-dom/client";
import { DraftEditor } from "../components/DraftEditor";
import { onOutcome } from "../ipc";
import { outcomeText } from "../messages";
import { useEvent } from "../useEvent";
import "../styles.css";

/** Review window: its capability allows only draft read/edit/copy/discard. */
function Review() {
  const [seq, setSeq] = useState(0);
  const [note, setNote] = useState("");
  const [error, setError] = useState("");
  useEvent(useCallback(() => onOutcome((o) => { setNote(outcomeText(o)); setSeq((n) => n + 1); }), []));
  const onError = useCallback((m: string) => { setError(m); }, []);
  return (
    <main>
      <h1>Review text</h1>
      <p className="hint">Automatic paste is turned off while this window is open. Copy the text, or focus the destination and use your paste shortcut.</p>
      {error && <p role="alert" className="error">{error}</p>}
      <DraftEditor refreshKey={seq} onError={onError} />
      <p role="status" className="hint">{note}</p>
    </main>
  );
}

const root = document.getElementById("review");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <Review />
    </StrictMode>,
  );
}
