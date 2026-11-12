import { StrictMode, useCallback, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { onHudBanner, onLevel, onPhase, type Outcome, type Phase } from "../ipc";
import { outcomeText, phaseLabel } from "../messages";
import { useEvent } from "../useEvent";
import "../styles.css";

/** HUD: phase, level, elapsed time and short banners. No commands, no text. */
function Hud() {
  const [phase, setPhase] = useState<Phase>("idle");
  const [since, setSince] = useState(() => Date.now());
  const [now, setNow] = useState(() => Date.now());
  const [level, setLevel] = useState(0);
  const [banner, setBanner] = useState<Outcome | null>(null);
  useEvent(useCallback(() => onPhase((p) => { setPhase(p); setSince(Date.now()); setBanner(null); if (p !== "listening") setLevel(0); }), []));
  useEvent(useCallback(() => onLevel(setLevel), []));
  useEvent(useCallback(() => onHudBanner(setBanner), []));
  useEffect(() => {
    const t = setInterval(() => { setNow(Date.now()); }, 500);
    return () => { clearInterval(t); };
  }, []);
  const secs = Math.max(0, Math.floor((now - since) / 1000));
  const listening = phase === "listening";
  const working = phase === "transcribing" || phase === "delivering" || phase === "finalizing";
  if (banner) {
    return <div className="hud-pill banner">{outcomeText(banner)}</div>;
  }
  return (
    <div className={`hud-pill ${phase}`}>
      <span>{listening ? "● " : ""}{phaseLabel[phase]}</span>
      {(listening || working) && <span className="hud-time">{secs}s</span>}
      {listening && (
        <span className="hud-meter" aria-hidden="true"><span style={{ width: `${String(Math.round(level * 100))}%` }} /></span>
      )}
    </div>
  );
}

const root = document.getElementById("hud");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <Hud />
    </StrictMode>,
  );
}
