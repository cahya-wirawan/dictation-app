import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { SettingsApp } from "./features/settings/SettingsApp";
import "./styles.css";

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <SettingsApp />
    </StrictMode>,
  );
}
