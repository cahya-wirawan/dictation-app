import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist", "src-tauri", "target", "vendor", "node_modules"] },
  {
    files: ["**/*.{ts,tsx}"],
    extends: [js.configs.recommended, ...tseslint.configs.strictTypeChecked],
    languageOptions: {
      globals: globals.browser,
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      // The frontend never reaches for these; the backend has no such APIs either.
      "no-restricted-globals": ["error", "fetch", "XMLHttpRequest", "WebSocket", "localStorage"],
      "no-restricted-properties": [
        "error",
        { object: "navigator", property: "mediaDevices", message: "Audio capture is native-only (docs/03 §1)." },
      ],
    },
  },
);
