import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { EVENTS } from "../ipc/events";

export interface RuntimeEventHandlers {
  onTaskChanged?: (taskId: string) => void;
  /// A run aggregate changed (started/completed — ④e). Payload is the run id.
  onRunChanged?: (runId: string) => void;
}

/// Tauri's unlisten (`@tauri-apps/api/event`) is `async () => _unlisten(...)` and
/// throws synchronously inside `unregisterListener` when the listener's internal
/// bookkeeping is already gone (the dev/StrictMode subscribe→teardown race). That
/// surfaces as an unhandled rejection because we fire-and-forget the cleanup. A
/// throw means the listener is already unregistered, so swallowing leaks nothing.
function safeUnlisten(u: () => void): void {
  try {
    const r = u() as unknown;
    if (r instanceof Promise) r.catch(() => {});
  } catch {
    /* listener already gone */
  }
}

/// Subscribes to backend-pushed Runtime events. v1 emits `task-changed` with the
/// affected task id; ④e adds `run-changed` with the affected run id. Either
/// handler is optional — the board passes both so cards AND lane/run state stay
/// fresh as the generator produces.
export function useRuntimeEvents({ onTaskChanged, onRunChanged }: RuntimeEventHandlers): void {
  useEffect(() => {
    let unlistens: Array<() => void> = [];
    let cancelled = false;
    (async () => {
      const a = await listen<string>(EVENTS.taskChanged, (e) => onTaskChanged?.(e.payload));
      const b = await listen<string>(EVENTS.runChanged, (e) => onRunChanged?.(e.payload));
      if (cancelled) {
        safeUnlisten(a);
        safeUnlisten(b);
      } else {
        unlistens = [a, b];
      }
    })();
    return () => {
      cancelled = true;
      unlistens.forEach(safeUnlisten);
    };
  }, [onTaskChanged, onRunChanged]);
}
