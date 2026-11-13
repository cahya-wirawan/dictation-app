import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { SessionNotice } from "./SessionNotice";

afterEach(cleanup);

describe("SessionNotice", () => {
  it("renders nothing off Linux", () => {
    const { container } = render(<SessionNotice session={null} />);
    expect(container.textContent).toBe("");
  });

  it("explains Wayland limits", () => {
    render(
      <SessionNotice
        session={{ kind: "wayland", desktop: "GNOME", globalShortcutsReliable: false, copyAvailable: true, autoPaste: false }}
      />,
    );
    const note = screen.getByRole("note");
    expect(note.textContent).toContain("Wayland, GNOME");
    expect(note.textContent).toContain("never pasted automatically");
    expect(note.textContent).toContain("Use Start in this window");
    expect(note.textContent).not.toContain("Copy is unavailable");
  });

  it("states when copy is unavailable", () => {
    render(
      <SessionNotice session={{ kind: "wayland", desktop: null, globalShortcutsReliable: false, copyAvailable: false, autoPaste: false }} />,
    );
    expect(screen.getByRole("note").textContent).toContain("Copy is unavailable");
  });
});
