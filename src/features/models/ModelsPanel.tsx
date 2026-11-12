import { useCallback, useEffect, useState } from "react";
import {
  activateModel,
  cancelModelOperation,
  getModels,
  importModel,
  installModel,
  onModelOpFinished,
  onModelProgress,
  removeModel,
  unloadModel,
  verifyModel,
  type ModelProgress,
  type ModelView,
} from "../../ipc";
import { useEvent } from "../../useEvent";

const mb = (n: number) => `${(n / 1_000_000).toFixed(0)} MB`;

function stateText(v: ModelView): string {
  switch (v.state.kind) {
    case "notInstalled": return "Not installed";
    case "partial": return `Download paused at ${mb(v.state.bytes)}`;
    case "installed": return v.state.verified ? "Installed and verified" : "Installed (checked on load)";
    case "invalid": return "Damaged — remove and reinstall";
  }
}

interface Props {
  onChanged: () => void;
  onError: (msg: string) => void;
}

/** Models from the bundled manifest only (docs/02 §3). No invented memory/latency figures. */
export function ModelsPanel({ onChanged, onError }: Props) {
  const [models, setModels] = useState<ModelView[]>([]);
  const [progress, setProgress] = useState<Record<string, ModelProgress>>({});
  const [activating, setActivating] = useState<string | null>(null);

  const reload = useCallback(() => {
    getModels().then(setModels).catch((e: unknown) => { onError(String(e)); });
  }, [onError]);
  useEffect(reload, [reload]);

  useEvent(useCallback(() => onModelProgress((p) => { setProgress((cur) => ({ ...cur, [p.id]: p })); }), []));
  useEvent(
    useCallback(
      () =>
        onModelOpFinished((f) => {
          setProgress((cur) => Object.fromEntries(Object.entries(cur).filter(([id]) => id !== f.id)));
          if (f.error && f.error !== "canceled") onError(`${f.op} failed: ${f.error}`);
          reload();
          onChanged();
        }),
      [reload, onChanged, onError],
    ),
  );

  const act = (p: Promise<unknown>) => {
    p.then(reload).then(onChanged).catch((e: unknown) => { onError(String(e)); });
  };

  return (
    <section aria-labelledby="models-h">
      <h2 id="models-h">Models</h2>
      <p className="hint">Models run entirely on this computer. Downloads come from Hugging Face and are checked against checksums built into the app.</p>
      <ul className="models">
        {models.map((v) => {
          const p = progress[v.model.id];
          const installed = v.state.kind === "installed";
          return (
            <li key={v.model.id}>
              <div>
                <strong>{v.model.displayName}</strong> {v.active && <span className="badge">In use</span>}
                <div className="hint">{mb(v.model.bytes)} · {v.model.profile === "balanced" ? "Balanced" : "Higher quality, slower"} · {stateText(v)}</div>
              </div>
              {v.busy && p && (
                <progress max={p.total} value={p.done} aria-label={`${v.model.displayName} progress`}>
                  {Math.round((p.done / p.total) * 100)}%
                </progress>
              )}
              <div className="row">
                {v.busy ? (
                  <button type="button" onClick={() => { act(cancelModelOperation(v.model.id)); }}>Cancel</button>
                ) : (
                  <>
                    {!installed && <button type="button" onClick={() => { act(installModel(v.model.id)); }}>{v.state.kind === "partial" ? "Resume download" : "Download"}</button>}
                    {!installed && <button type="button" onClick={() => { act(importModel(v.model.id)); }}>Import file…</button>}
                    {installed && !v.active && (
                      <button
                        type="button"
                        disabled={activating !== null}
                        onClick={() => {
                          setActivating(v.model.id);
                          activateModel(v.model.id)
                            .then(reload)
                            .then(onChanged)
                            .catch((e: unknown) => { onError(`Could not load model: ${String(e)}`); })
                            .finally(() => { setActivating(null); });
                        }}
                      >
                        {activating === v.model.id ? "Loading…" : "Use this model"}
                      </button>
                    )}
                    {installed && v.state.kind === "installed" && !v.state.verified && (
                      <button type="button" onClick={() => { act(verifyModel(v.model.id)); }}>Verify</button>
                    )}
                    {v.active && <button type="button" onClick={() => { act(unloadModel()); }}>Unload</button>}
                    {(installed || v.state.kind !== "notInstalled") && !v.active && (
                      <button type="button" onClick={() => { act(removeModel(v.model.id)); }}>Remove</button>
                    )}
                  </>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
