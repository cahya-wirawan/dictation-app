import { StrictMode, useCallback, useState } from "react";
import { createRoot } from "react-dom/client";
import { onLevel, onPhase, type Phase } from "../ipc";
import { phaseLabel } from "../messages";
import { useEvent } from "../useEvent";
import "../styles.css";

/** HUD receives phase and level events only; its capability grants no commands. */
function Hud() {
  const [phase, setPhase] = useState<Phase>("idle");
  const [level, setLevel] = useState(0);
  useEvent(useCallback(() => onPhase((p) => { setPhase(p); if (p !== "listening") setLevel(0); }), []));
  useEvent(useCallback(() => onLevel(setLevel), []));
  const listening = phase === "listening";
  return (
    <div className={`hud-pill ${phase}`}>
      <span>{listening ? "● " : ""}{phaseLabel[phase]}</span>
      {listening && (
        <span className="hud-meter" aria-hidden="true">
          <span style={{ width: `${String(Math.round(level * 100))}%` }} />
        </span>
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
