import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { EVENTS } from "../ipc/events";

export interface RuntimeEventHandlers {
  onTaskChanged?: (taskId: string) => void;
}

/// Subscribes to backend-pushed Runtime events. v1 emits `task.changed` with
/// the affected task id as payload (the app's first Tauri event channel).
export function useRuntimeEvents({ onTaskChanged }: RuntimeEventHandlers): void {
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const un = await listen<string>(EVENTS.taskChanged, (event) => {
        onTaskChanged?.(event.payload);
      });
      if (cancelled) un();
      else unlisten = un;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [onTaskChanged]);
}
