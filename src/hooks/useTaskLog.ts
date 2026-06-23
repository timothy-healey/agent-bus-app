import { useCallback, useEffect, useRef, useState } from "react";
import { onTaskLog } from "../ipc/runtime";

/// Accumulates display-only worker log fragments (`task.log`) into a per-task
/// buffer the CardDrawer's "live log" tab renders. The authoritative settled
/// state still arrives via `task.changed`; this is purely for live feel.
export function useTaskLog() {
  // A ref holds the canonical buffers (mutated on every fragment); a version
  // counter triggers re-render so `logFor` returns fresh text without rebuilding
  // a new object on every fragment.
  const buffers = useRef<Record<string, string>>({});
  const [, setVersion] = useState(0);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const un = await onTaskLog((log) => {
        buffers.current[log.task_id] = (buffers.current[log.task_id] ?? "") + log.delta;
        setVersion((v) => v + 1);
      });
      if (cancelled) un();
      else unlisten = un;
    })();
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  const logFor = useCallback((taskId: string): string => {
    return buffers.current[taskId] ?? "";
  }, []);

  return { logFor };
}
