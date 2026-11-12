import { useState } from "react";
import { getDiagnostics, saveDiagnostics } from "../../ipc";

function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : "Unknown error";
}

/**
 * Support report (docs/10 §5): preview first, then an explicit Save. The file
 * is written by Rust through a native save dialog; nothing is uploaded.
 */
export function SupportReport() {
  const [preview, setPreview] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);

  const load = () => {
    setStatus("");
    getDiagnostics().then(setPreview).catch((e: unknown) => { setStatus(errorText(e)); });
  };
  const save = () => {
    setBusy(true);
    setStatus("");
    saveDiagnostics()
      .then((saved) => { setStatus(saved ? "Report saved." : "Not saved."); })
      .catch((e: unknown) => { setStatus(errorText(e)); })
      .finally(() => { setBusy(false); });
  };

  return (
    <section aria-labelledby="support-h">
      <h3 id="support-h">Support report</h3>
      <p className="hint">
        Versions, settings values, permission and error codes, and timings. It never contains what you said, audio,
        app or window names, device names or file paths. Nothing is sent anywhere; you decide where to save it.
      </p>
      <div className="row">
        <button type="button" onClick={load}>{preview === null ? "Preview report" : "Refresh preview"}</button>
        <button type="button" disabled={preview === null || busy} onClick={save}>Save report…</button>
      </div>
      {preview !== null && (
        <pre className="report" tabIndex={0} aria-label="Support report preview">{preview}</pre>
      )}
      <p role="status" className="hint">{status}</p>
    </section>
  );
}
