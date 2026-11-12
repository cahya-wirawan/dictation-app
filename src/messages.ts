// User-facing copy (docs/02 §5). Never includes spoken text.
import type { AppError, DraftReason, Outcome, Phase } from "./ipc";

export const phaseLabel: Record<Phase, string> = {
  idle: "Ready",
  starting: "Starting…",
  listening: "Listening…",
  finalizing: "Finishing…",
  transcribing: "Transcribing locally…",
  delivering: "Delivering…",
  draftReady: "Text ready",
  recovering: "Stopping…",
};

export function errorText(e: AppError): string {
  switch (e) {
    case "MODEL_UNAVAILABLE": return "Install or import a local model to begin.";
    case "MODEL_INVALID": return "The installed model failed verification.";
    case "MIC_DENIED": return "Microphone access is disabled.";
    case "MIC_DISCONNECTED": return "Your selected microphone disconnected. Recording stopped.";
    case "AUDIO_OVERFLOW": return "Audio could not be processed in time. Recording stopped.";
    case "INFERENCE_TIMEOUT": return "Local transcription took too long.";
    case "WORKER_CRASH": return "The local transcription engine stopped unexpectedly.";
    case "TARGET_CHANGED": return "Text ready. The focused app changed, so automatic paste was skipped.";
    case "PASTE_UNSUPPORTED": return "Automatic paste is unavailable here. Your text is ready to copy.";
    case "CLIPBOARD_BUSY": return "Text ready. The clipboard is busy.";
    case "INPUT_UNCERTAIN": return "The paste may not have completed. Check the destination before resending.";
    case "DRAFT_PENDING": return "Review, copy, or discard your previous text first.";
    case "BUSY": return "Dictation is already in progress.";
    case "SHORTCUT_CONFLICT": return "The shortcut is used by another app.";
    case "STALE_REQUEST": return "That text changed. Try again.";
    case "SETTINGS_CONFLICT": return "Settings changed elsewhere. Reloaded.";
    case "NOT_READY": return "Dictation is not ready yet.";
  }
}

export function draftReasonText(r: DraftReason): string {
  if (typeof r === "object") return errorText(r.blocked);
  switch (r) {
    case "reviewMode": return "Text ready to copy.";
    case "reviewOpened": return "Text ready. Automatic paste was skipped because review was open.";
    case "uncertain": return "The paste may not have completed. Check the destination before resending.";
  }
}

export function outcomeText(o: Outcome): string {
  if (typeof o === "object") {
    if ("draftReady" in o) return draftReasonText(o.draftReady);
    if ("failed" in o) return errorText(o.failed);
    return errorText(o.rejected);
  }
  switch (o) {
    case "pasteSent": return "Paste sent.";
    case "noSpeech": return "No speech detected.";
    case "noUsableText": return "No usable text was recognized. Try speaking closer to the microphone.";
    case "canceled": return "Canceled.";
    case "cancelTooLate": return "The text had already been sent when cancel arrived.";
    case "draftCopied": return "Copied to the clipboard.";
    case "draftDiscarded": return "Text discarded.";
    case "draftExpired": return "The unsent text expired and was cleared.";
  }
}
