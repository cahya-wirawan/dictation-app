import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Review } from "./Review";
import "../styles.css";

const root = document.getElementById("review");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <Review />
    </StrictMode>,
  );
}
