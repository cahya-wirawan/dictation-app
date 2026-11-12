import { useEffect } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

/** Subscribe to a Tauri event for the component lifetime. */
export function useEvent(subscribe: () => Promise<UnlistenFn>): void {
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    void subscribe().then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [subscribe]);
}
