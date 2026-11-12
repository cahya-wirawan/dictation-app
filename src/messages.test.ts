import { describe, expect, it } from "vitest";
import type { AppError, Outcome } from "./ipc";
import { errorText, outcomeText, phaseLabel } from "./messages";

const ERRORS: AppError[] = [
  "NOT_READY", "BUSY", "DRAFT_PENDING", "SHORTCUT_CONFLICT", "MIC_DENIED", "MIC_DISCONNECTED", "AUDIO_OVERFLOW",
  "MODEL_INVALID", "MODEL_UNAVAILABLE", "WORKER_CRASH", "INFERENCE_TIMEOUT", "TARGET_CHANGED", "MODIFIERS_HELD",
  "PASTE_UNSUPPORTED", "CLIPBOARD_BUSY", "INPUT_UNCERTAIN", "STALE_REQUEST", "SETTINGS_CONFLICT",
];

describe("user-facing copy (docs/02 §5)", () => {
  it("has text for every error code", () => {
    for (const e of ERRORS) expect(errorText(e).length, e).toBeGreaterThan(5);
  });

  it("uses the specified wording for key states", () => {
    expect(errorText("MODEL_UNAVAILABLE")).toBe("Install or import a local model to begin.");
    expect(errorText("MIC_DISCONNECTED")).toBe("Your selected microphone disconnected. Recording stopped.");
    expect(outcomeText("noSpeech")).toBe("No speech detected.");
    expect(outcomeText("pasteSent")).toBe("Paste sent.");
    expect(outcomeText({ draftReady: { blocked: "TARGET_CHANGED" } })).toBe(
      "Text ready. The focused app changed, so automatic paste was skipped.",
    );
  });

  it("covers every outcome shape and phase", () => {
    const outcomes: Outcome[] = [
      "pasteSent", "noSpeech", "noUsableText", "canceled", "cancelTooLate", "draftCopied", "draftDiscarded", "draftExpired",
      { draftReady: "reviewMode" }, { draftReady: "reviewOpened" }, { draftReady: "uncertain" },
      { failed: "WORKER_CRASH" }, { rejected: "BUSY" },
    ];
    for (const o of outcomes) expect(outcomeText(o)).toMatch(/\w/);
    expect(Object.keys(phaseLabel)).toHaveLength(8);
  });
});
