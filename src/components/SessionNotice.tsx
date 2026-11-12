import type { SessionInfo } from "../ipc";

/**
 * Linux only: states what this desktop session can do, from the Rust probe
 * (docs/05 §1). Nothing is promised that was not measured.
 */
export function SessionNotice({ session }: { session: SessionInfo | null }) {
  if (!session) return null;
  const name = session.kind === "wayland" ? "Wayland" : session.kind === "x11" ? "X11" : "no graphical session";
  return (
    <div className="hint" role="note" aria-label="Desktop session limits">
      <p>
        Linux ({name}{session.desktop ? `, ${session.desktop}` : ""}) is experimental. Text is never pasted
        automatically: when dictation finishes, review it and copy it into your app.
      </p>
      {!session.globalShortcutsReliable && (
        <p>The global shortcut may not work in this session. Use Start in this window instead.</p>
      )}
      {!session.copyAvailable && <p>Copy is unavailable here (no X11 or XWayland clipboard).</p>}
    </div>
  );
}
