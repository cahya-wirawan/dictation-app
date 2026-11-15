import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Outcome } from "../ipc";

const api = vi.hoisted(() => {
  const listeners: ((o: Outcome) => void)[] = [];
  return {
    listeners,
    onOutcome: vi.fn((cb: (o: Outcome) => void) => {
      listeners.push(cb);
      return Promise.resolve(() => undefined);
    }),
    getDraft: vi.fn(() => Promise.resolve({ id: "d1", revision: 0, text: "hello", reason: "reviewMode", attempted: false, expiresInMs: 60_000 })),
    editDraft: vi.fn(),
    copyDraft: vi.fn(),
    discardDraft: vi.fn(),
  };
});
vi.mock("../ipc", () => api);

import { Review } from "./Review";

const emit = (o: Outcome) => { act(() => { for (const l of api.listeners) l(o); }); };

describe("Review", () => {
  afterEach(() => { cleanup(); api.listeners.length = 0; });

  it("explains a refused start and clears the notice on the next outcome", async () => {
    render(<Review />);
    await waitFor(() => { expect(api.listeners.length).toBeGreaterThan(0); });
    expect(screen.queryByText(/can't start while this text is waiting/)).toBeNull();
    emit({ rejected: "DRAFT_PENDING" });
    expect(screen.getByText(/can't start while this text is waiting/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Copy" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Discard" })).toBeTruthy();
    emit("draftCopied");
    expect(screen.queryByText(/can't start while this text is waiting/)).toBeNull();
  });
});
