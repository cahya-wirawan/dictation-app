import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { ShortcutField } from "../../components/ShortcutField";
import { DraftEditor } from "../../components/DraftEditor";
import {
  checkPermissions,
  getSettings,
  getSnapshot,
  listInputDevices,
  onMicTestEnded,
  onMicTestLevel,
  onOutcome,
  onPhase,
  onProbeReport,
  openPrivacySettings,
  openReview,
  requestPermission,
  runWorkerSelftest,
  setLaunchAtLogin,
  startMicTest,
  stopMicTest,
  unloadModel,
  updateSettings,
  type Config,
  type InputDevice,
  type Outcome,
  type Permissions,
  type Phase,
  type ProbeReport,
  type SelftestReport,
  type SettingsPatch,
  type Snapshot,
} from "../../ipc";
import { setLocale } from "../../i18n";
import { useEvent } from "../../useEvent";
import { DictationPanel } from "../dictation/DictationPanel";
import { ModelsPanel } from "../models/ModelsPanel";
import { Onboarding } from "../onboarding/Onboarding";
import { SupportReport } from "./SupportReport";

const PAGES = ["general", "recording", "shortcuts", "models", "delivery", "privacy", "about"] as const;
type Page = (typeof PAGES)[number];
const PAGE_LABEL: Record<Page, string> = {
  general: "General",
  recording: "Recording",
  shortcuts: "Shortcuts",
  models: "Transcription",
  delivery: "Delivery",
  privacy: "Privacy",
  about: "About",
};

const LANGUAGES: [string, string][] = [["auto", "Detect automatically"], ["en", "English"], ["de", "German"], ["id", "Indonesian"]];

function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : "Unknown error";
}

/** Settings and status window. Snapshots are authoritative; events only hint. */
export function SettingsApp() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [cfg, setCfg] = useState<Config | null>(null);
  const [phase, setPhase] = useState<Phase | null>(null);
  const [lastOutcome, setLastOutcome] = useState<Outcome | null>(null);
  const [seq, setSeq] = useState(0);
  const [page, setPage] = useState<Page>("general");
  const [error, setError] = useState("");
  const tabs = useRef<Record<Page, HTMLButtonElement | null>>({} as Record<Page, HTMLButtonElement | null>);

  const refresh = useCallback(() => {
    getSnapshot()
      .then((s) => { setSnapshot(s); setPhase(s.phase); })
      .catch((e: unknown) => { setError(errorText(e)); });
    getSettings()
      .then((c) => { setCfg(c); setLocale(c.ui.locale); })
      .catch((e: unknown) => { setError(errorText(e)); });
  }, []);
  const reportError = useCallback((m: string) => { setError(m); }, []);
  useEffect(refresh, [refresh]);

  useEvent(useCallback(() => onPhase((p) => { setPhase(p); setSeq((n) => n + 1); }), []));
  useEvent(useCallback(() => onOutcome((o) => { setLastOutcome(o); setSeq((n) => n + 1); refresh(); }), [refresh]));

  const patch = (p: SettingsPatch) => {
    if (!cfg) return;
    updateSettings(cfg.revision, p)
      .then(setCfg)
      .catch((e: unknown) => { setError(errorText(e)); refresh(); });
  };

  if (!snapshot || !cfg) return <main><p>Loading…</p></main>;
  if (!snapshot.onboardingComplete) {
    return (
      <Onboarding
        snapshot={snapshot}
        config={cfg}
        refresh={refresh}
        onConfig={setCfg}
        onDone={refresh}
        onError={reportError}
      />
    );
  }

  const onTabKey = (e: KeyboardEvent<HTMLButtonElement>) => {
    const i = PAGES.indexOf(page);
    const next = e.key === "ArrowDown" || e.key === "ArrowRight" ? i + 1 : e.key === "ArrowUp" || e.key === "ArrowLeft" ? i - 1 : null;
    if (next === null) return;
    e.preventDefault();
    const p = PAGES[(next + PAGES.length) % PAGES.length] ?? "general";
    setPage(p);
    tabs.current[p]?.focus();
  };
  const mac = snapshot.platform === "macos";

  return (
    <main className="settings">
      <h1>Local Dictation</h1>
      {error && (
        <p role="alert" className="error">
          {error} <button type="button" onClick={() => { setError(""); }}>Dismiss</button>
        </p>
      )}
      {snapshot.settingsNote && <p role="alert" className="error">{snapshot.settingsNote}</p>}
      <DictationPanel snapshot={snapshot} phase={phase} lastOutcome={lastOutcome} onError={reportError} />
      {phase === "draftReady" && (
        <section aria-labelledby="draft-h">
          <h2 id="draft-h">Your text</h2>
          <DraftEditor refreshKey={seq} onError={reportError} />
        </section>
      )}

      <div className="pages">
        <div role="tablist" aria-label="Settings" aria-orientation="vertical" className="tabs">
          {PAGES.map((p) => (
            <button
              key={p}
              ref={(el) => { tabs.current[p] = el; }}
              role="tab"
              id={`tab-${p}`}
              aria-selected={page === p}
              aria-controls={`panel-${p}`}
              tabIndex={page === p ? 0 : -1}
              onClick={() => { setPage(p); }}
              onKeyDown={onTabKey}
            >
              {PAGE_LABEL[p]}
            </button>
          ))}
        </div>
        <div role="tabpanel" id={`panel-${page}`} aria-labelledby={`tab-${page}`} className="panel">
          {page === "general" && <General cfg={cfg} patch={patch} setCfg={setCfg} onError={reportError} />}
          {page === "recording" && <Recording cfg={cfg} patch={patch} onError={reportError} />}
          {page === "shortcuts" && (
            <div className="form">
              <ShortcutField slot="toggle" label="Start / finish dictation" value={cfg.shortcuts.toggle} mac={mac} onSaved={refresh} />
              <ShortcutField slot="cancel" label="Cancel dictation" value={cfg.shortcuts.cancel} mac={mac} onSaved={refresh} />
              <ShortcutField slot="pasteDraft" label="Paste unsent text into the focused field (optional)" value={cfg.shortcuts.paste_draft} mac={mac} optional onSaved={refresh} />
              {snapshot.shortcuts.error && <p className="error" role="alert">{snapshot.shortcuts.error}</p>}
            </div>
          )}
          {page === "models" && (
            <div className="form">
              <label>
                Language
                <select value={cfg.transcription.language} onChange={(e) => { patch({ transcription: { language: e.target.value } }); }}>
                  {LANGUAGES.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
                </select>
              </label>
              <p className="hint">For very short phrases, choosing the language explicitly is more reliable than detection.</p>
              <ModelsPanel onChanged={refresh} onError={reportError} />
              {snapshot.activeModel && (
                <button type="button" onClick={() => { unloadModel().then(refresh).catch((e: unknown) => { reportError(errorText(e)); }); }}>
                  Unload model from memory
                </button>
              )}
            </div>
          )}
          {page === "delivery" && <Delivery cfg={cfg} patch={patch} onError={reportError} />}
          {page === "privacy" && <Privacy />}
          {page === "about" && <About snapshot={snapshot} />}
        </div>
      </div>
    </main>
  );
}

interface PageProps {
  cfg: Config;
  patch: (p: SettingsPatch) => void;
  onError: (m: string) => void;
}

function General({ cfg, patch, setCfg, onError }: PageProps & { setCfg: (c: Config) => void }) {
  return (
    <div className="form">
      <label className="check">
        <input type="checkbox" checked={cfg.ui.show_hud} onChange={(e) => { patch({ ui: { show_hud: e.target.checked } }); }} />
        Show the status indicator while dictating
      </label>
      <label className="check">
        <input type="checkbox" checked={cfg.ui.reduced_motion} onChange={(e) => { patch({ ui: { reduced_motion: e.target.checked } }); }} />
        Reduce motion
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={cfg.ui.launch_at_login}
          onChange={(e) => {
            setLaunchAtLogin(e.target.checked)
              .then(() => getSettings())
              .then(setCfg)
              .catch((err: unknown) => { onError(errorText(err)); });
          }}
        />
        Open Local Dictation when I log in
      </label>
      <button type="button" onClick={() => { void openReview(); }}>Open review window</button>
    </div>
  );
}

function Recording({ cfg, patch, onError }: PageProps) {
  const [devices, setDevices] = useState<InputDevice[]>([]);
  const [testing, setTesting] = useState(false);
  const [level, setLevel] = useState(0);
  const [note, setNote] = useState("");
  useEffect(() => { listInputDevices().then(setDevices).catch((e: unknown) => { setNote(errorText(e)); }); }, []);
  useEffect(() => () => { void stopMicTest(); }, []);
  useEvent(useCallback(() => onMicTestLevel((l) => { setLevel(l.level); }), []));
  useEvent(useCallback(() => onMicTestEnded((r) => { setTesting(false); setLevel(0); if (r !== "stopped") setNote(`Test ended: ${r}`); }), []));
  return (
    <div className="form">
      <label>
        Microphone
        <select value={cfg.audio.device_id ?? ""} onChange={(e) => { patch({ audio: { device_id: e.target.value || null } }); }}>
          <option value="">System default (confirmed at each start)</option>
          {devices.map((d) => <option key={d.id} value={d.id}>{d.name}{d.isDefault ? " (default)" : ""}</option>)}
        </select>
      </label>
      <p className="hint">If the selected microphone disconnects, recording stops; another microphone is never used silently.</p>
      <div>
        <span className="visually-hidden" id="mic-meter-label">Input level (test only)</span>
        <div className="meter" aria-hidden="true"><div style={{ width: `${String(Math.round(level * 100))}%` }} /></div>
        <button
          type="button"
          aria-describedby="mic-meter-label"
          onClick={() => {
            const p = testing ? stopMicTest() : startMicTest(cfg.audio.device_id);
            p.then(() => { setTesting(!testing); }).catch((e: unknown) => { onError(errorText(e)); });
          }}
        >
          {testing ? "Stop test" : "Test microphone"}
        </button>
        {note && <p role="status">{note}</p>}
      </div>
      <label>
        Finish after silence: {(cfg.audio.silence_ms / 1000).toFixed(1)} s
        <input type="range" min={500} max={2500} step={100} value={cfg.audio.silence_ms}
          onChange={(e) => { patch({ audio: { silence_ms: Number(e.target.value) } }); }} />
      </label>
      <label>
        Give up if no speech within: {(cfg.audio.no_speech_timeout_ms / 1000).toFixed(0)} s
        <input type="range" min={3000} max={15000} step={1000} value={cfg.audio.no_speech_timeout_ms}
          onChange={(e) => { patch({ audio: { no_speech_timeout_ms: Number(e.target.value) } }); }} />
      </label>
      <p className="hint">Changes apply to the next recording.</p>
    </div>
  );
}

function Delivery({ cfg, patch, onError }: PageProps) {
  const [perm, setPerm] = useState<Permissions | null>(null);
  const [newApp, setNewApp] = useState("");
  const recheck = useCallback(() => { checkPermissions().then(setPerm).catch((e: unknown) => { onError(errorText(e)); }); }, [onError]);
  useEffect(recheck, [recheck]);
  const ax = perm?.accessibility;
  return (
    <div className="form">
      <label className="check">
        <input type="checkbox" checked={cfg.delivery.mode === "auto_paste"}
          onChange={(e) => { patch({ delivery: { mode: e.target.checked ? "auto_paste" : "copy_only" } }); }} />
        Automatically paste into the app I was using (otherwise: review and copy)
      </label>
      {ax && ax !== "notApplicable" && (
        <p role="status" className={ax === "granted" ? "hint" : "error"}>
          Accessibility: {ax === "granted" ? "enabled" : "not enabled — automatic paste falls back to copy"}.
          {ax !== "granted" && (
            <>
              {" "}
              <button type="button" onClick={() => { requestPermission("accessibility").then(() => openPrivacySettings("accessibility")).then(recheck).catch((e: unknown) => { onError(errorText(e)); }); }}>
                Enable automatic paste
              </button>{" "}
              <button type="button" onClick={recheck}>Check again</button>
            </>
          )}
        </p>
      )}
      {ax === "notApplicable" && <p className="hint">Automatic paste is not available on this system yet; text is kept for you to copy.</p>}
      <p className="hint">
        Automatic paste puts the text on the clipboard and sends one paste command. It is skipped if you switched apps or
        fields, in password fields, and in terminals. The dictated text stays on the clipboard afterwards.
      </p>
      <fieldset>
        <legend>Never paste automatically into these apps</legend>
        <ul>
          {cfg.delivery.excluded_apps.map((a) => (
            <li key={a}>
              <code>{a}</code>{" "}
              <button type="button" aria-label={`Remove ${a}`} onClick={() => { patch({ delivery: { excluded_apps: cfg.delivery.excluded_apps.filter((x) => x !== a) } }); }}>Remove</button>
            </li>
          ))}
        </ul>
        <label>
          App identifier (for example com.apple.Notes)
          <input value={newApp} onChange={(e) => { setNewApp(e.target.value.trim()); }} />
        </label>
        <button type="button" disabled={!newApp} onClick={() => { patch({ delivery: { excluded_apps: [...cfg.delivery.excluded_apps, newApp] } }); setNewApp(""); }}>
          Add exclusion
        </button>
        <p className="hint">Terminal apps are always excluded.</p>
      </fieldset>
    </div>
  );
}

function Privacy() {
  return (
    <div className="prose">
      <h2>What happens to your speech</h2>
      <ul>
        <li>The microphone is open only while you dictate or test it. Audio stays in memory and is never written to disk.</li>
        <li>Transcription runs on this computer. Nothing is sent to a server; the only network use is downloading a model when you ask.</li>
        <li>Unsent text is kept in memory for up to 10 minutes and cleared when you quit or the screen locks. There is no history.</li>
        <li>Pasted or copied text stays on the system clipboard, where clipboard history, sync and other apps can read it.</li>
        <li>No analytics, and no logs containing what you said.</li>
      </ul>
    </div>
  );
}

function About({ snapshot }: { snapshot: Snapshot }) {
  const [report, setReport] = useState<SelftestReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [probe, setProbe] = useState<ProbeReport | null>(snapshot.lastProbe);
  useEvent(useCallback(() => onProbeReport(setProbe), []));
  return (
    <div className="prose">
      <dl>
        <dt>Version</dt><dd>{snapshot.appVersion}</dd>
        <dt>Platform</dt><dd>{snapshot.platform}</dd>
        <dt>Active model</dt><dd>{snapshot.activeModel ?? "none"}</dd>
        <dt>Speech engine</dt><dd>{report?.engineRevision ?? "whisper.cpp v1.9.5 (bundled)"}</dd>
      </dl>
      <p className="hint">Built with whisper.cpp (MIT), OpenAI Whisper model weights (MIT), Tauri (MIT/Apache-2.0), CPAL, rubato and earshot (MIT/Apache-2.0).</p>
      <details>
        <summary>Diagnostics</summary>
        <button type="button" disabled={busy} onClick={() => {
          setBusy(true); setErr("");
          runWorkerSelftest().then(setReport).catch((e: unknown) => { setErr(errorText(e)); }).finally(() => { setBusy(false); });
        }}>
          {busy ? "Running…" : "Test the local engine"}
        </button>
        {err && <p className="error">{err}</p>}
        {report && (
          <dl>
            <dt>Load</dt><dd>{report.loadMs} ms</dd>
            <dt>Inference</dt><dd>{report.inferenceMs.join(" / ")} ms for {report.audioMs / 1000} s of test audio</dd>
            <dt>Test text</dt><dd>{report.fixtureText}</dd>
          </dl>
        )}
        {probe && <p className="hint">Last focus check: {probe.afterHudCheck}</p>}
      </details>
      <SupportReport />
    </div>
  );
}
