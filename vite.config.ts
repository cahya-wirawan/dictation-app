import { resolve } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed dev port and no browser auto-open.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  build: {
    target: "es2022",
    rolldownOptions: {
      input: {
        main: resolve(import.meta.dirname, "index.html"),
        hud: resolve(import.meta.dirname, "hud.html"),
        review: resolve(import.meta.dirname, "review.html"),
      },
    },
  },
});
