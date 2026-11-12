// Typed wrappers for the M0 command/event contract. Rust serde types in
// src-tauri/src/lib.rs are the source of truth; generation from Rust replaces
// this hand-written file in M1 (docs/04 §3).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ProbeReport {
  before: string;
  afterHudCheck: string;
  autoDeliveryWouldBeAllowed: boolean;
}

export interface Snapshot {
  schemaVersion: 1;
  appVersion: string;
  platform: string;
  accessibility: "Granted" | "Denied" | "NotApplicable";
  workerFound: boolean;
  modelInstalled: boolean;
  modelPath: string;
  modelSourceUrl: string;
  shortcuts: { toggle: string; cancel: string; error: string | null };
  micTestActive: boolean;
  lastProbe: ProbeReport | null;
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

export type HudPhase = "idle" | "listening";

export const getSnapshot = () => invoke<Snapshot>("get_snapshot");
export const listInputDevices = () => invoke<InputDevice[]>("list_input_devices");
export const startMicTest = (deviceId: string | null) => invoke<null>("start_mic_test", { deviceId });
export const stopMicTest = () => invoke<null>("stop_mic_test");
export const runWorkerSelftest = () => invoke<SelftestReport>("run_worker_selftest");

export const onMicLevel = (cb: (l: MicLevel) => void): Promise<UnlistenFn> =>
  listen<MicLevel>("mic_level", (e) => { cb(e.payload); });
export const onMicTestEnded = (cb: (reason: string) => void): Promise<UnlistenFn> =>
  listen<{ reason: string }>("mic_test_ended", (e) => { cb(e.payload.reason); });
export const onProbeReport = (cb: (r: ProbeReport) => void): Promise<UnlistenFn> =>
  listen<ProbeReport>("probe_report", (e) => { cb(e.payload); });
export const onHudPhase = (cb: (p: HudPhase) => void): Promise<UnlistenFn> =>
  listen<{ phase: HudPhase }>("phase_changed", (e) => { cb(e.payload.phase); });
