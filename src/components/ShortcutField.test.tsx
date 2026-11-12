import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  setShortcut: vi.fn((_slot: string, chord: string | null) => Promise.resolve({ chord })),
  pauseShortcuts: vi.fn(() => Promise.resolve(null)),
  resumeShortcuts: vi.fn(() => Promise.resolve(null)),
}));
vi.mock("../ipc", () => api);

import { ShortcutField } from "./ShortcutField";

describe("ShortcutField", () => {
  afterEach(cleanup);

  it("pauses global shortcuts while focused and saves only a valid chord", async () => {
    const onSaved = vi.fn();
    render(<ShortcutField slot="toggle" label="Toggle" value="Ctrl+Alt+Space" mac onSaved={onSaved} />);
    const input = screen.getByLabelText<HTMLInputElement>("Toggle");
    input.focus();
    expect(api.pauseShortcuts).toHaveBeenCalled();
    fireEvent.keyDown(input, { code: "KeyA" });
    expect(screen.getByRole("status").textContent).toContain("Include Control");
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "Save shortcut" }).disabled).toBe(true);
    fireEvent.keyDown(input, { code: "KeyD", ctrlKey: true, altKey: true });
    expect(input.value).toBe("⌃⌥D");
    fireEvent.click(screen.getByRole("button", { name: "Save shortcut" }));
    await waitFor(() => { expect(onSaved).toHaveBeenCalledWith("Ctrl+Alt+KeyD"); });
    expect(api.setShortcut).toHaveBeenCalledWith("toggle", "Ctrl+Alt+KeyD");
    input.blur();
    expect(api.resumeShortcuts).toHaveBeenCalled();
  });

  it("reports an OS registration conflict instead of pretending success", async () => {
    api.setShortcut.mockImplementationOnce(() => Promise.reject(new Error("SHORTCUT_CONFLICT: taken")));
    const onSaved = vi.fn();
    render(<ShortcutField slot="toggle" label="Toggle" value="Ctrl+Alt+Space" mac={false} onSaved={onSaved} />);
    const input = screen.getByLabelText<HTMLInputElement>("Toggle");
    fireEvent.keyDown(input, { code: "Space", metaKey: true });
    fireEvent.click(screen.getByRole("button", { name: "Save shortcut" }));
    await waitFor(() => { expect(screen.getByRole("status").textContent).toContain("already uses this shortcut"); });
    expect(onSaved).not.toHaveBeenCalled();
  });

  it("keeps plain Tab for keyboard navigation", async () => {
    const user = userEvent.setup();
    render(<ShortcutField slot="toggle" label="Toggle" value="Ctrl+Alt+Space" mac onSaved={vi.fn()} />);
    screen.getByLabelText("Toggle").focus();
    await user.tab();
    expect(document.activeElement).not.toBe(screen.getByLabelText("Toggle"));
  });
});
