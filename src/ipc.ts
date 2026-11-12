// Typed wrappers for the command/event contract. Rust serde types in
// src-tauri/src/lib.rs and crates/domain are the source of truth; generating
// this file from Rust is a tracked follow-up (docs/04 §3).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Phase =
  | "idle"
  | "starting"
  | "listening"
  | "finalizing"
  | "transcribing"
  | "delivering"
  | "draftReady"
  | "recovering";

export type AppError =
  | "NOT_READY" | "BUSY" | "DRAFT_PENDING" | "SHORTCUT_CONFLICT" | "MIC_DENIED"
  | "MIC_DISCONNECTED" | "AUDIO_OVERFLOW" | "MODEL_INVALID" | "MODEL_UNAVAILABLE"
  | "WORKER_CRASH" | "INFERENCE_TIMEOUT" | "TARGET_CHANGED" | "PASTE_UNSUPPORTED"
  | "CLIPBOARD_BUSY" | "INPUT_UNCERTAIN" | "STALE_REQUEST" | "SETTINGS_CONFLICT";

export type DraftReason = "reviewMode" | "reviewOpened" | "uncertain" | { blocked: AppError };

/** Never carries transcript text. */
export type Outcome =
  | "pasteSent" | "noSpeech" | "noUsableText" | "canceled" | "cancelTooLate"
  | "draftCopied" | "draftDiscarded" | "draftExpired"
  | { draftReady: DraftReason }
  | { failed: AppError }
  | { rejected: AppError };

export interface ProbeReport {
  before: string;
  afterHudCheck: string;
  autoDeliveryWouldBeAllowed: boolean;
}

export interface Snapshot {
  schemaVersion: 1;
  appVersion: string;
  platform: string;
  phase: Phase | null;
  draft: { id: string; revision: number; attempted: boolean } | null;
  readyError: AppError | null;
  accessibility: "Granted" | "Denied" | "NotApplicable";
  workerFound: boolean;
  activeModel: string | null;
  settingsRevision: number;
  settingsNote: string | null;
  shortcuts: { toggle: string; cancel: string; error: string | null };
  micTestActive: boolean;
  lastProbe: ProbeReport | null;
}

export interface DraftView {
  id: string;
  revision: number;
  text: string;
  reason: DraftReason;
  attempted: boolean;
}

export interface Config {
  schema_version: 1;
  revision: number;
  audio: { device_id: string | null; silence_ms: number; no_speech_timeout_ms: number; max_capture_ms: number };
  transcription: { model_id: string; language: string; backend: "auto" | "cpu" | "metal" };
  delivery: { mode: "auto_paste" | "copy_only"; clipboard_policy: "leave_text" | "preserve"; excluded_apps: string[] };
  shortcuts: { toggle: string; cancel: string; paste_draft: string | null };
  ui: { show_hud: boolean; launch_at_login: boolean; locale: string };
}

/** JSON merge-patch over Config (validated again in Rust). */
export type SettingsPatch = { [K in keyof Config]?: Partial<Config[K]> };

export type ModelState =
  | { kind: "notInstalled" }
  | { kind: "partial"; bytes: number }
  | { kind: "installed"; verified: boolean }
  | { kind: "invalid" };

export interface ModelView {
  model: {
    id: string;
    displayName: string;
    profile: "balanced" | "quality";
    fileName: string;
    bytes: number;
    engineFormat: string;
    multilingual: boolean;
    license: string;
    source: string;
  };
  state: ModelState;
  active: boolean;
  busy: boolean;
}

export interface ModelProgress {
  id: string;
  done: number;
  total: number;
}

export interface ModelOpFinished {
  id: string;
  op: "install" | "import" | "verify";
  error: string | null;
}

export interface InputDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

export interface SelftestReport {
  workerPath: string;
  engineRevision: string;
  backends: string[];
  modelId: string;
  spawnMs: number;
  loadMs: number;
  inferenceMs: number[];
  audioMs: number;
  language: string | null;
  fixtureText: string;
  workerReaped: boolean;
}

export interface MicLevel {
  level: number;
  sampleRate: number;
  channels: number;
}

export const getSnapshot = () => invoke<Snapshot>("get_snapshot");
export const startDictation = () => invoke<null>("start_dictation");
export const stopDictation = () => invoke<null>("stop_dictation");
export const cancelDictation = () => invoke<null>("cancel_dictation");
export const getDraft = () => invoke<DraftView | null>("get_draft");
export const editDraft = (id: string, expectedRevision: number, text: string) =>
  invoke<null>("edit_draft", { id, expectedRevision, text });
export const copyDraft = (id: string, revision: number) => invoke<null>("copy_draft", { id, revision });
export const discardDraft = (id: string) => invoke<null>("discard_draft", { id });
export const getSettings = () => invoke<Config>("get_settings");
export const updateSettings = (expectedRevision: number, patch: SettingsPatch) =>
  invoke<Config>("update_settings", { expectedRevision, patch });
export const getModels = () => invoke<ModelView[]>("get_models");
export const installModel = (modelId: string) => invoke<null>("install_model", { modelId });
export const importModel = (modelId: string) => invoke<null>("import_model", { modelId });
export const verifyModel = (modelId: string) => invoke<null>("verify_model", { modelId });
export const cancelModelOperation = (modelId: string) => invoke<null>("cancel_model_operation", { modelId });
export const removeModel = (modelId: string) => invoke<null>("remove_model", { modelId });
export const activateModel = (modelId: string) => invoke<null>("activate_model", { modelId });
export const unloadModel = () => invoke<null>("unload_model");
export const listInputDevices = () => invoke<InputDevice[]>("list_input_devices");
export const startMicTest = (deviceId: string | null) => invoke<null>("start_mic_test", { deviceId });
export const stopMicTest = () => invoke<null>("stop_mic_test");
export const runWorkerSelftest = () => invoke<SelftestReport>("run_worker_selftest");

export const onPhase = (cb: (p: Phase) => void): Promise<UnlistenFn> =>
  listen<{ phase: Phase }>("phase_changed", (e) => { cb(e.payload.phase); });
export const onOutcome = (cb: (o: Outcome) => void): Promise<UnlistenFn> =>
  listen<{ outcome: Outcome }>("outcome", (e) => { cb(e.payload.outcome); });
export const onLevel = (cb: (level: number) => void): Promise<UnlistenFn> =>
  listen<{ level: number }>("mic_level", (e) => { cb(e.payload.level); });
export const onMicTestLevel = (cb: (l: MicLevel) => void): Promise<UnlistenFn> =>
  listen<MicLevel>("mic_level", (e) => { cb(e.payload); });
export const onMicTestEnded = (cb: (reason: string) => void): Promise<UnlistenFn> =>
  listen<{ reason: string }>("mic_test_ended", (e) => { cb(e.payload.reason); });
export const onProbeReport = (cb: (r: ProbeReport) => void): Promise<UnlistenFn> =>
  listen<ProbeReport>("probe_report", (e) => { cb(e.payload); });
export const onModelProgress = (cb: (p: ModelProgress) => void): Promise<UnlistenFn> =>
  listen<ModelProgress>("model_progress", (e) => { cb(e.payload); });
export const onModelOpFinished = (cb: (f: ModelOpFinished) => void): Promise<UnlistenFn> =>
  listen<ModelOpFinished>("model_op_finished", (e) => { cb(e.payload); });
