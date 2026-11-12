import { useCallback, useEffect, useState } from "react";
import {
  getSnapshot,
  listInputDevices,
  onMicTestEnded,
  onMicTestLevel,
  onOutcome,
  onPhase,
  onProbeReport,
  runWorkerSelftest,
  startMicTest,
  stopMicTest,
  type InputDevice,
  type Outcome,
  type Phase,
  type ProbeReport,
  type SelftestReport,
  type Snapshot,
} from "../../ipc";
import { useEvent } from "../../useEvent";
import { DictationPanel } from "../dictation/DictationPanel";
import { DraftPanel } from "../draft/DraftPanel";
import { ModelsPanel } from "../models/ModelsPanel";
import { SettingsForm } from "./SettingsForm";

function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : "Unknown error";
}

/** Settings and review window. Snapshots are authoritative; events only hint. */
export function SettingsApp() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [phase, setPhase] = useState<Phase | null>(null);
  const [lastOutcome, setLastOutcome] = useState<Outcome | null>(null);
  const [outcomeSeq, setOutcomeSeq] = useState(0);
  const [devices, setDevices] = useState<InputDevice[]>([]);
  const [level, setLevel] = useState(0);
  const [micActive, setMicActive] = useState(false);
  const [micNote, setMicNote] = useState("");
  const [selftest, setSelftest] = useState<SelftestReport | null>(null);
  const [selftestBusy, setSelftestBusy] = useState(false);
  const [probe, setProbe] = useState<ProbeReport | null>(null);
  const [error, setError] = useState("");

  const refresh = useCallback(() => {
    getSnapshot()
      .then((s) => {
        setSnapshot(s);
        setPhase(s.phase);
        setMicActive(s.micTestActive);
        setProbe(s.lastProbe);
      })
      .catch((e: unknown) => { setError(errorText(e)); });
  }, []);
  const reportError = useCallback((msg: string) => { setError(msg); }, []);

  useEffect(() => {
    refresh();
    listInputDevices().then(setDevices).catch((e: unknown) => { setMicNote(errorText(e)); });
  }, [refresh]);

  useEvent(useCallback(() => onPhase((p) => { setPhase(p); }), []));
  useEvent(
    useCallback(
      () =>
        onOutcome((o) => {
          setLastOutcome(o);
          setOutcomeSeq((n) => n + 1);
          refresh();
        }),
      [refresh],
    ),
  );
  useEvent(useCallback(() => onMicTestLevel((l) => { setLevel(l.level); }), []));
  useEvent(
    useCallback(
      () =>
        onMicTestEnded((reason) => {
          setMicActive(false);
          setLevel(0);
          setMicNote(reason === "stopped" ? "" : `Mic test ended: ${reason}`);
        }),
      [],
    ),
  );
  useEvent(useCallback(() => onProbeReport(setProbe), []));

  const toggleMic = () => {
    setMicNote("");
    const action = micActive ? stopMicTest() : startMicTest(null);
    action
      .then(() => { setMicActive(!micActive); })
      .catch((e: unknown) => { setMicNote(errorText(e)); });
  };

  const selfTest = () => {
    setSelftestBusy(true);
    setError("");
    runWorkerSelftest()
      .then(setSelftest)
      .catch((e: unknown) => { setError(errorText(e)); })
      .finally(() => { setSelftestBusy(false); });
  };

  return (
    <main>
      <h1>Local Dictation <small>{snapshot ? `v${snapshot.appVersion}` : ""}</small></h1>
      {error && <p role="alert" className="error">{error} <button type="button" onClick={() => { setError(""); }}>Dismiss</button></p>}
      {snapshot?.settingsNote && <p role="alert" className="error">{snapshot.settingsNote}</p>}

      <DictationPanel snapshot={snapshot} phase={phase} lastOutcome={lastOutcome} onError={reportError} />
      <DraftPanel phase={phase} outcomeSeq={outcomeSeq} onError={reportError} />
      <ModelsPanel onChanged={refresh} onError={reportError} />
      <SettingsForm devices={devices} onError={reportError} />

      <details>
        <summary>Diagnostics</summary>
        <section aria-labelledby="status-h">
          <h2 id="status-h">Status</h2>
          {snapshot && (
            <dl>
              <dt>Platform</dt><dd>{snapshot.platform}</dd>
              <dt>Accessibility</dt><dd>{snapshot.accessibility}</dd>
              <dt>Worker binary</dt><dd>{snapshot.workerFound ? "found" : "missing"}</dd>
              <dt>Active model</dt><dd>{snapshot.activeModel ?? "none"}</dd>
              <dt>Shortcuts</dt>
              <dd>{snapshot.shortcuts.error ? <span className="error">{snapshot.shortcuts.error}</span> : "registered"}</dd>
            </dl>
          )}
          <button type="button" onClick={refresh}>Refresh</button>
        </section>

        <section aria-labelledby="mic-h">
          <h2 id="mic-h">Microphone test</h2>
          <div className="meter" role="meter" aria-label="Input level" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(level * 100)}>
            <div style={{ width: `${String(Math.round(level * 100))}%` }} />
          </div>
          <button type="button" onClick={toggleMic}>{micActive ? "Stop test" : "Start test"}</button>
          <p className="hint">The microphone is open only while the test runs (max 30 s). Nothing is recorded.</p>
          {micNote && <p role="status">{micNote}</p>}
        </section>

        <section aria-labelledby="worker-h">
          <h2 id="worker-h">Local engine self-test</h2>
          <button type="button" onClick={selfTest} disabled={selftestBusy}>
            {selftestBusy ? "Running…" : "Transcribe test fixture"}
          </button>
          {selftest && (
            <dl>
              <dt>Engine</dt><dd>{selftest.engineRevision} ({selftest.backends.join(", ")})</dd>
              <dt>Model</dt><dd>{selftest.modelId}, load {selftest.loadMs} ms</dd>
              <dt>Inference</dt><dd>{selftest.inferenceMs.join(" / ")} ms for {selftest.audioMs / 1000} s audio</dd>
              <dt>Fixture text</dt><dd>{selftest.fixtureText}</dd>
              <dt>Worker reaped</dt><dd>{selftest.workerReaped ? "yes" : "no"}</dd>
            </dl>
          )}
        </section>

        <section aria-labelledby="probe-h">
          <h2 id="probe-h">HUD focus probe</h2>
          <p className="hint">Each toggle press from another app compares its app, window and field before and after the HUD appears.</p>
          {probe ? (
            <dl>
              <dt>Result</dt><dd><strong>{probe.afterHudCheck}</strong>{probe.autoDeliveryWouldBeAllowed ? " (auto-paste would be allowed)" : " (draft fallback)"}</dd>
              <dt>Target</dt><dd><code>{probe.before}</code></dd>
            </dl>
          ) : (
            <p>No probe yet.</p>
          )}
        </section>
      </details>
    </main>
  );
}
