import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Config, Permissions, Snapshot } from "../../ipc";

const ipc = vi.hoisted(() => {
  const perms: Permissions = { microphone: "notDetermined", accessibility: "denied" };
  return { perms };
});

vi.mock("../../ipc", () => {
  const noop = () => Promise.resolve(() => undefined);
  return {
    checkPermissions: vi.fn(() => Promise.resolve(ipc.perms)),
    requestPermission: vi.fn((k: "microphone" | "accessibility") => {
      ipc.perms = { ...ipc.perms, [k]: "denied" };
      return Promise.resolve("denied");
    }),
    openPrivacySettings: vi.fn(() => Promise.resolve(true)),
    completeOnboarding: vi.fn(() => Promise.resolve(null)),
    setLaunchAtLogin: vi.fn(() => Promise.resolve(false)),
    startMicTest: vi.fn(() => Promise.resolve(null)),
    stopMicTest: vi.fn(() => Promise.resolve(null)),
    startDictation: vi.fn(() => Promise.resolve(null)),
    stopDictation: vi.fn(() => Promise.resolve(null)),
    getDraft: vi.fn(() => Promise.resolve(null)),
    discardDraft: vi.fn(() => Promise.resolve(null)),
    updateSettings: vi.fn(() => Promise.resolve(null)),
    getModels: vi.fn(() => Promise.resolve([])),
    setShortcut: vi.fn(() => Promise.resolve({ chord: "Ctrl+Alt+Space" })),
    pauseShortcuts: vi.fn(() => Promise.resolve(null)),
    resumeShortcuts: vi.fn(() => Promise.resolve(null)),
    onMicTestLevel: noop,
    onOutcome: noop,
    onModelProgress: noop,
    onModelOpFinished: noop,
  };
});

import { Onboarding } from "./Onboarding";

const snapshot = (over: Partial<Snapshot> = {}): Snapshot => ({
  schemaVersion: 1, appVersion: "0.3.0", platform: "macos", phase: "idle", draft: null, readyError: null,
  accessibility: "Denied", workerFound: true, activeModel: null, settingsRevision: 1, settingsNote: null,
  onboardingComplete: false, shortcuts: { toggle: "Ctrl+Alt+Space", cancel: "Ctrl+Alt+Escape", error: null },
  micTestActive: false, lastProbe: null, session: null, ...over,
});

const config = {
  schema_version: 3, revision: 1,
  audio: { device_id: null, silence_ms: 1000, no_speech_timeout_ms: 8000, max_capture_ms: 120000, continuous: false },
  transcription: { model_id: "whisper-base-multilingual", language: "auto", backend: "auto" },
  delivery: { mode: "auto_paste", clipboard_policy: "leave_text", excluded_apps: [] },
  shortcuts: { toggle: "Ctrl+Alt+Space", cancel: "Ctrl+Alt+Escape", paste_draft: null },
  ui: { show_hud: true, launch_at_login: false, locale: "en", onboarding_complete: false, reduced_motion: false },
} satisfies Config;

function renderOnb(s: Snapshot) {
  const onDone = vi.fn();
  render(<Onboarding snapshot={s} config={config} refresh={vi.fn()} onConfig={vi.fn()} onDone={onDone} onError={vi.fn()} />);
  return { onDone };
}

describe("Onboarding (keyboard only)", () => {
  beforeEach(() => { ipc.perms = { microphone: "notDetermined", accessibility: "denied" }; });
  afterEach(cleanup);

  it("moves focus to each step heading and is operable with Tab/Enter", async () => {
    const user = userEvent.setup();
    renderOnb(snapshot());
    await waitFor(() => { expect(document.activeElement?.textContent).toBe("Dictate into your apps"); });
    await user.tab();
    expect(document.activeElement?.textContent).toBe("Continue");
    await user.keyboard("{Enter}");
    await waitFor(() => { expect(document.activeElement?.textContent).toBe("Microphone"); });
  });

  it("cannot skip the microphone step and offers recovery after denial", async () => {
    const user = userEvent.setup();
    renderOnb(snapshot());
    await user.click(screen.getByRole("button", { name: "Continue" }));
    const cont = await screen.findByRole<HTMLButtonElement>("button", { name: "Continue" });
    expect(cont.disabled).toBe(true);
    await user.click(await screen.findByRole("button", { name: "Allow microphone" }));
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open System Settings" })).toBeTruthy();
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "Continue" }).disabled).toBe(true);
    // The user enables it in System Settings and checks again.
    ipc.perms = { ...ipc.perms, microphone: "granted" };
    await user.click(screen.getByRole("button", { name: "Check again" }));
    await waitFor(() => { expect(screen.getByRole<HTMLButtonElement>("button", { name: "Continue" }).disabled).toBe(false); });
  });

  it("requires an active model before continuing", async () => {
    ipc.perms = { microphone: "granted", accessibility: "granted" };
    const user = userEvent.setup();
    renderOnb(snapshot({ activeModel: null }));
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(await screen.findByRole("button", { name: "Continue" }));
    expect(await screen.findByText("Install a model to continue.")).toBeTruthy();
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "Continue" }).disabled).toBe(true);
  });
});
