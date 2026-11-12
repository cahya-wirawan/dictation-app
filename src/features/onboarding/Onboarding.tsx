import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { ShortcutField } from "../../components/ShortcutField";
import { t } from "../../i18n";
import {
  checkPermissions,
  completeOnboarding,
  discardDraft,
  getDraft,
  onMicTestLevel,
  onOutcome,
  openPrivacySettings,
  requestPermission,
  setLaunchAtLogin,
  startDictation,
  startMicTest,
  stopDictation,
  stopMicTest,
  updateSettings,
  type Config,
  type Permissions,
  type Snapshot,
} from "../../ipc";
import { outcomeText } from "../../messages";
import { useEvent } from "../../useEvent";
import { ModelsPanel } from "../models/ModelsPanel";

const STEPS = ["welcome", "microphone", "model", "shortcut", "paste", "trial", "done"] as const;
type Step = (typeof STEPS)[number];

interface Props {
  snapshot: Snapshot;
  config: Config;
  refresh: () => void;
  onConfig: (c: Config) => void;
  onDone: () => void;
  onError: (m: string) => void;
}

/** First-run setup (docs/02 §2). Each step can only advance when it is really done. */
export function Onboarding({ snapshot, config, refresh, onConfig, onDone, onError }: Props) {
  const [step, setStep] = useState<Step>("welcome");
  const [perm, setPerm] = useState<Permissions | null>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const mac = snapshot.platform === "macos";

  const recheck = useCallback(() => {
    checkPermissions().then(setPerm).catch((e: unknown) => { onError(String(e)); });
  }, [onError]);
  useEffect(recheck, [recheck, step]);
  // Move focus to the new step's heading for screen-reader and keyboard users.
  useEffect(() => { heading.current?.focus(); }, [step]);
  // Re-check when the user returns from System Settings.
  useEffect(() => {
    const onFocus = () => { recheck(); refresh(); };
    window.addEventListener("focus", onFocus);
    return () => { window.removeEventListener("focus", onFocus); };
  }, [recheck, refresh]);

  const index = STEPS.indexOf(step);
  const go = (d: 1 | -1) => { setStep(STEPS[Math.min(STEPS.length - 1, Math.max(0, index + d))] ?? "welcome"); };

  const canContinue: Record<Step, boolean> = {
    welcome: true,
    microphone: perm?.microphone === "granted" || perm?.microphone === "notApplicable",
    model: snapshot.activeModel !== null,
    shortcut: snapshot.shortcuts.toggle !== "" && snapshot.shortcuts.error === null,
    paste: true,
    trial: true,
    done: true,
  };

  return (
    <main className="onboarding" aria-labelledby="onb-h">
      <p className="hint">Step {index + 1} of {STEPS.length}</p>
      <section>
        {step === "welcome" && (
          <>
            <h1 id="onb-h" ref={heading} tabIndex={-1}>{t("onb.welcome.title")}</h1>
            <p>{t("onb.welcome.body")}</p>
            <p className="hint">{t("onb.welcome.clipboard")}</p>
          </>
        )}
        {step === "microphone" && <MicStep heading={heading} perm={perm} recheck={recheck} onError={onError} />}
        {step === "model" && (
          <>
            <h1 id="onb-h" ref={heading} tabIndex={-1}>{t("onb.model.title")}</h1>
            <p>{t("onb.model.body")}</p>
            <ModelsPanel onChanged={refresh} onError={onError} />
            {!canContinue.model && <p className="hint">{t("onb.model.needed")}</p>}
          </>
        )}
        {step === "shortcut" && (
          <>
            <h1 id="onb-h" ref={heading} tabIndex={-1}>{t("onb.shortcut.title")}</h1>
            <p>{t("onb.shortcut.body")}</p>
            <ShortcutField slot="toggle" label="Start / finish dictation" value={config.shortcuts.toggle} mac={mac} onSaved={refresh} />
            {snapshot.shortcuts.error && <p className="error" role="alert">{snapshot.shortcuts.error}</p>}
          </>
        )}
        {step === "paste" && (
          <PasteStep heading={heading} perm={perm} recheck={recheck} config={config} onConfig={onConfig} onError={onError} />
        )}
        {step === "trial" && <TrialStep heading={heading} onError={onError} />}
        {step === "done" && (
          <>
            <h1 id="onb-h" ref={heading} tabIndex={-1}>{t("onb.done.title")}</h1>
            <p>{t("onb.done.body")}</p>
            <p>Shortcut: <kbd>{snapshot.shortcuts.toggle}</kbd></p>
            <label className="check">
              <input
                type="checkbox"
                checked={config.ui.launch_at_login}
                onChange={(e) => {
                  setLaunchAtLogin(e.target.checked)
                    .then((on) => { onConfig({ ...config, ui: { ...config.ui, launch_at_login: on } }); })
                    .catch((err: unknown) => { onError(String(err)); });
                }}
              />
              {t("onb.done.launch")}
            </label>
          </>
        )}
      </section>
      <nav className="row" aria-label="Setup steps">
        {index > 0 && <button type="button" onClick={() => { go(-1); }}>{t("nav.back")}</button>}
        {step !== "done" ? (
          <button type="button" disabled={!canContinue[step]} onClick={() => { go(1); }}>{t("nav.continue")}</button>
        ) : (
          <button
            type="button"
            onClick={() => { completeOnboarding().then(onDone).catch((e: unknown) => { onError(String(e)); }); }}
          >
            {t("nav.finish")}
          </button>
        )}
      </nav>
    </main>
  );
}

interface StepProps {
  heading: RefObject<HTMLHeadingElement | null>;
  onError: (m: string) => void;
}

function MicStep({ heading, perm, recheck, onError }: StepProps & { perm: Permissions | null; recheck: () => void }) {
  const [testing, setTesting] = useState(false);
  const [level, setLevel] = useState(0);
  useEvent(useCallback(() => onMicTestLevel((l) => { setLevel(l.level); }), []));
  // The microphone test closes when leaving this step (docs/02 §2).
  useEffect(() => () => { void stopMicTest(); }, []);
  const status = perm?.microphone;
  return (
    <>
      <h1 id="onb-h" ref={heading} tabIndex={-1}>{t("onb.mic.title")}</h1>
      <p>{t("onb.mic.body")}</p>
      {status === "notDetermined" && (
        <button type="button" onClick={() => { requestPermission("microphone").then(recheck).catch((e: unknown) => { onError(String(e)); }); }}>
          {t("onb.mic.allow")}
        </button>
      )}
      {status === "denied" && (
        <div role="alert">
          <p className="error">{t("onb.mic.denied")}</p>
          <div className="row">
            <button type="button" onClick={() => { void openPrivacySettings("microphone"); }}>{t("settings.openSettings")}</button>
            <button type="button" onClick={recheck}>{t("onb.paste.recheck")}</button>
          </div>
        </div>
      )}
      {status === "restricted" && <p className="error" role="alert">{t("onb.mic.restricted")}</p>}
      {(status === "granted" || status === "notApplicable") && (
        <>
          <div className="meter" aria-hidden="true"><div style={{ width: `${String(Math.round(level * 100))}%` }} /></div>
          <button
            type="button"
            onClick={() => {
              const p = testing ? stopMicTest() : startMicTest(null);
              p.then(() => { setTesting(!testing); setLevel(0); }).catch((e: unknown) => { onError(String(e)); });
            }}
          >
            {testing ? t("onb.mic.stopTest") : t("onb.mic.test")}
          </button>
        </>
      )}
    </>
  );
}

function PasteStep({
  heading,
  perm,
  recheck,
  config,
  onConfig,
  onError,
}: StepProps & { perm: Permissions | null; recheck: () => void; config: Config; onConfig: (c: Config) => void }) {
  const setMode = (mode: Config["delivery"]["mode"]) => {
    updateSettings(config.revision, { delivery: { mode } }).then(onConfig).catch((e: unknown) => { onError(String(e)); });
  };
  const ax = perm?.accessibility;
  return (
    <>
      <h1 id="onb-h" ref={heading} tabIndex={-1}>{t("onb.paste.title")}</h1>
      {ax === "notApplicable" ? (
        <p>{t("onb.paste.unavailable")}</p>
      ) : (
        <>
          <p>{t("onb.paste.body")}</p>
          {ax === "granted" ? (
            <p role="status">{t("onb.paste.granted")}</p>
          ) : (
            <p className="hint" role="status">{t("onb.paste.pending")}</p>
          )}
          <div className="row">
            {ax !== "granted" && (
              <button
                type="button"
                onClick={() => {
                  setMode("auto_paste");
                  requestPermission("accessibility")
                    .then(() => openPrivacySettings("accessibility"))
                    .then(recheck)
                    .catch((e: unknown) => { onError(String(e)); });
                }}
              >
                {t("onb.paste.enable")}
              </button>
            )}
            {ax !== "granted" && <button type="button" onClick={recheck}>{t("onb.paste.recheck")}</button>}
            <button type="button" aria-pressed={config.delivery.mode === "copy_only"} onClick={() => { setMode("copy_only"); }}>
              {t("onb.paste.copyOnly")}
            </button>
          </div>
        </>
      )}
    </>
  );
}

function TrialStep({ heading, onError }: StepProps) {
  const [running, setRunning] = useState(false);
  const [text, setText] = useState("");
  const [note, setNote] = useState("");
  useEvent(
    useCallback(
      () =>
        onOutcome((o) => {
          setRunning(false);
          setNote(outcomeText(o));
          if (typeof o === "object" && "draftReady" in o) {
            // The trial result is shown here, then the draft is cleared.
            getDraft()
              .then((d) => {
                if (d) {
                  setText(d.text);
                  return discardDraft(d.id);
                }
                return null;
              })
              .catch((e: unknown) => { onError(String(e)); });
          }
        }),
      [onError],
    ),
  );
  return (
    <>
      <h1 id="onb-h" ref={heading} tabIndex={-1}>{t("onb.trial.title")}</h1>
      <p>{t("onb.trial.body")}</p>
      <div className="row">
        <button type="button" disabled={running} onClick={() => { setRunning(true); setNote(""); startDictation().catch((e: unknown) => { setRunning(false); onError(String(e)); }); }}>
          {t("onb.trial.start")}
        </button>
        <button type="button" disabled={!running} onClick={() => { void stopDictation(); }}>{t("onb.trial.finish")}</button>
      </div>
      <label htmlFor="trial-text">{t("onb.trial.result")}</label>
      <textarea id="trial-text" rows={3} readOnly value={text} />
      <p role="status" className="hint">{note}</p>
    </>
  );
}
