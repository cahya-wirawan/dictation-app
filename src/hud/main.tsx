import { StrictMode, useCallback, useState } from "react";
import { createRoot } from "react-dom/client";
import { onHudPhase, type HudPhase } from "../ipc";
import { useEvent } from "../useEvent";
import "../styles.css";

/** HUD receives phase events only; its capability grants no commands. */
function Hud() {
  const [phase, setPhase] = useState<HudPhase>("idle");
  useEvent(useCallback(() => onHudPhase(setPhase), []));
  return <div className={`hud-pill ${phase}`}>{phase === "listening" ? "● Listening…" : "Idle"}</div>;
}

const root = document.getElementById("hud");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <Hud />
    </StrictMode>,
  );
}
