import { cancelDictation, startDictation, stopDictation, type Outcome, type Phase, type Snapshot } from "../../ipc";
import { errorText, outcomeText, phaseLabel } from "../../messages";

interface Props {
  snapshot: Snapshot | null;
  phase: Phase | null;
  lastOutcome: Outcome | null;
  onError: (msg: string) => void;
}

function asText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : "Unknown error";
}

/** Status and explicit controls. The global shortcut is the primary path. */
export function DictationPanel({ snapshot, phase, lastOutcome, onError }: Props) {
  const busy = phase !== null && phase !== "idle" && phase !== "draftReady";
  const listening = phase === "starting" || phase === "listening";
  const run = (p: Promise<unknown>) => { p.catch((e: unknown) => { onError(asText(e)); }); };
  return (
    <section aria-labelledby="dictation-h">
      <h2 id="dictation-h">Dictation</h2>
      <p className="status" role="status">
        <strong>{phase ? phaseLabel[phase] : "Unavailable"}</strong>
        {lastOutcome && !busy && <span> · {outcomeText(lastOutcome)}</span>}
      </p>
      {snapshot?.readyError && <p className="error">{errorText(snapshot.readyError)}</p>}
      {snapshot && (
        <p className="hint">
          Press <kbd>{snapshot.shortcuts.toggle}</kbd> in any app to start; press again or pause to finish.
          <kbd>{snapshot.shortcuts.cancel}</kbd> cancels.
        </p>
      )}
      <div className="row">
        <button type="button" onClick={() => { run(startDictation()); }} disabled={busy || snapshot?.readyError != null}>Start</button>
        <button type="button" onClick={() => { run(stopDictation()); }} disabled={!listening}>Finish</button>
        <button type="button" onClick={() => { run(cancelDictation()); }} disabled={!busy}>Cancel</button>
      </div>
      <p className="hint">Automatic paste is not enabled in this build; finished text appears below for you to copy.</p>
    </section>
  );
}
