import { useCallback, useEffect, useState } from "react";
import {
  getSnapshot,
  listInputDevices,
  onMicLevel,
  onMicTestEnded,
  onProbeReport,
  runWorkerSelftest,
  startMicTest,
  stopMicTest,
  type InputDevice,
  type ProbeReport,
  type SelftestReport,
  type Snapshot,
} from "../../ipc";
import { useEvent } from "../../useEvent";

function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : "Unknown error";
}

/** M0 settings stub: status, mic test, worker self-test, HUD probe results. */
export function SettingsApp() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [devices, setDevices] = useState<InputDevice[]>([]);
  const [deviceId, setDeviceId] = useState<string | null>(null);
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
        setMicActive(s.micTestActive);
        setProbe(s.lastProbe);
      })
      .catch((e: unknown) => { setError(errorText(e)); });
  }, []);

  useEffect(() => {
    refresh();
    listInputDevices().then(setDevices).catch((e: unknown) => { setMicNote(errorText(e)); });
  }, [refresh]);

  useEvent(useCallback(() => onMicLevel((l) => { setLevel(l.level); }), []));
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
    const action = micActive ? stopMicTest() : startMicTest(deviceId);
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
      <h1>Local Dictation <small>M0 spike</small></h1>
      {error && <p role="alert" className="error">{error}</p>}

      <section aria-labelledby="status-h">
        <h2 id="status-h">Status</h2>
        {snapshot ? (
          <dl>
            <dt>Platform</dt><dd>{snapshot.platform}</dd>
            <dt>Accessibility</dt><dd>{snapshot.accessibility}</dd>
            <dt>Worker binary</dt><dd>{snapshot.workerFound ? "found" : "missing"}</dd>
            <dt>Model</dt><dd>{snapshot.modelInstalled ? "installed" : "not installed"} <code>{snapshot.modelPath}</code></dd>
            <dt>Shortcuts</dt>
            <dd>
              Toggle <kbd>{snapshot.shortcuts.toggle}</kbd>, cancel <kbd>{snapshot.shortcuts.cancel}</kbd>
              {snapshot.shortcuts.error && <span className="error"> {snapshot.shortcuts.error}</span>}
            </dd>
          </dl>
        ) : (
          <p>Loading…</p>
        )}
        <button type="button" onClick={refresh}>Refresh</button>
      </section>

      <section aria-labelledby="mic-h">
        <h2 id="mic-h">Microphone test</h2>
        <label>
          Input device{" "}
          <select
            value={deviceId ?? ""}
            onChange={(e) => { setDeviceId(e.target.value || null); }}
            disabled={micActive}
          >
            <option value="">System default (resolved to a specific device)</option>
            {devices.map((d) => (
              <option key={d.id} value={d.id}>{d.name}{d.isDefault ? " (default)" : ""}</option>
            ))}
          </select>
        </label>
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
            <dt>Language</dt><dd>{selftest.language ?? "unknown"}</dd>
            <dt>Fixture text</dt><dd>{selftest.fixtureText}</dd>
            <dt>Worker reaped</dt><dd>{selftest.workerReaped ? "yes" : "no"}</dd>
          </dl>
        )}
      </section>

      <section aria-labelledby="probe-h">
        <h2 id="probe-h">HUD focus probe</h2>
        <p className="hint">
          Focus a text field in another app, then press the toggle shortcut. The HUD appears and the app,
          window and focused field are compared before and after.
        </p>
        {probe ? (
          <dl>
            <dt>Result</dt><dd><strong>{probe.afterHudCheck}</strong>{probe.autoDeliveryWouldBeAllowed ? " (auto-paste would be allowed)" : " (draft fallback)"}</dd>
            <dt>Target</dt><dd><code>{probe.before}</code></dd>
          </dl>
        ) : (
          <p>No probe yet.</p>
        )}
      </section>
    </main>
  );
}
