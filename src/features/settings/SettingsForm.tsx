import { useEffect, useState } from "react";
import { getSettings, updateSettings, type Config, type InputDevice, type SettingsPatch } from "../../ipc";

const LANGUAGES: [string, string][] = [
  ["auto", "Detect automatically"],
  ["en", "English"],
  ["de", "German"],
  ["id", "Indonesian"],
];

interface Props {
  devices: InputDevice[];
  onError: (msg: string) => void;
}

/** Settings apply to the next capture; Rust re-validates every field. */
export function SettingsForm({ devices, onError }: Props) {
  const [cfg, setCfg] = useState<Config | null>(null);

  useEffect(() => {
    getSettings().then(setCfg).catch((e: unknown) => { onError(String(e)); });
  }, [onError]);

  if (!cfg) return null;

  const patch = (p: SettingsPatch) => {
    updateSettings(cfg.revision, p)
      .then(setCfg)
      .catch((e: unknown) => {
        onError(String(e));
        getSettings().then(setCfg).catch(() => undefined); // reconcile after conflicts
      });
  };

  return (
    <section aria-labelledby="settings-h">
      <h2 id="settings-h">Settings</h2>
      <div className="form">
        <label>
          Microphone
          <select
            value={cfg.audio.device_id ?? ""}
            onChange={(e) => { patch({ audio: { device_id: e.target.value || null } }); }}
          >
            <option value="">System default (confirmed at each start)</option>
            {devices.map((d) => (
              <option key={d.id} value={d.id}>{d.name}{d.isDefault ? " (default)" : ""}</option>
            ))}
          </select>
        </label>
        <label>
          Language
          <select value={cfg.transcription.language} onChange={(e) => { patch({ transcription: { language: e.target.value } }); }}>
            {LANGUAGES.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
          </select>
        </label>
        <label>
          Finish after silence: {(cfg.audio.silence_ms / 1000).toFixed(1)} s
          <input
            type="range" min={500} max={2500} step={100}
            value={cfg.audio.silence_ms}
            onChange={(e) => { patch({ audio: { silence_ms: Number(e.target.value) } }); }}
          />
        </label>
        <label>
          Give up if no speech within: {(cfg.audio.no_speech_timeout_ms / 1000).toFixed(0)} s
          <input
            type="range" min={3000} max={15000} step={1000}
            value={cfg.audio.no_speech_timeout_ms}
            onChange={(e) => { patch({ audio: { no_speech_timeout_ms: Number(e.target.value) } }); }}
          />
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={cfg.delivery.mode === "auto_paste"}
            onChange={(e) => { patch({ delivery: { mode: e.target.checked ? "auto_paste" : "copy_only" } }); }}
          />
          Automatically paste into the app I was using (otherwise: review and copy)
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={cfg.shortcuts.paste_draft !== null}
            onChange={(e) => { patch({ shortcuts: { paste_draft: e.target.checked ? "Ctrl+Alt+V" : null } }); }}
          />
          Paste unsent text into the focused field with <kbd>Ctrl+Option+V</kbd>
        </label>
        <p className="hint">
          Automatic paste puts the text on the clipboard and sends one paste command. It is skipped if you switched
          apps or fields, in password fields, and in terminals.
        </p>
        <label className="check">
          <input type="checkbox" checked={cfg.ui.show_hud} onChange={(e) => { patch({ ui: { show_hud: e.target.checked } }); }} />
          Show the status indicator while dictating
        </label>
      </div>
    </section>
  );
}
