import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  getDiagnostics: vi.fn(() => Promise.resolve('{ "reportVersion": 1 }')),
  saveDiagnostics: vi.fn(() => Promise.resolve(true)),
}));
vi.mock("../../ipc", () => ipc);

import { SupportReport } from "./SupportReport";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("SupportReport", () => {
  it("requires a preview before saving and saves only on explicit action", async () => {
    const user = userEvent.setup();
    render(<SupportReport />);
    const save = screen.getByRole("button", { name: "Save report…" });
    expect((save as HTMLButtonElement).disabled).toBe(true);
    expect(ipc.getDiagnostics).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Preview report" }));
    expect((await screen.findByLabelText("Support report preview")).textContent).toContain('"reportVersion": 1');
    expect(ipc.saveDiagnostics).not.toHaveBeenCalled();

    await user.click(save);
    await waitFor(() => { expect(screen.getByRole("status").textContent).toBe("Report saved."); });
    expect(ipc.saveDiagnostics).toHaveBeenCalledTimes(1);
  });

  it("reports a canceled save", async () => {
    ipc.saveDiagnostics.mockResolvedValueOnce(false);
    const user = userEvent.setup();
    render(<SupportReport />);
    await user.click(screen.getByRole("button", { name: "Preview report" }));
    await user.click(await screen.findByRole("button", { name: "Save report…" }));
    await waitFor(() => { expect(screen.getByRole("status").textContent).toBe("Not saved."); });
  });
});
