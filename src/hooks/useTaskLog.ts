import { useCallback, useEffect, useRef, useState } from "react";
import { onTaskLog } from "../ipc/runtime";

/// One contiguous run of same-kind log text (B). An ordered list of these is what
/// the CardDrawer renders — output as prose, thinking dimmed/italic.
export interface LogSegment {
  kind: "output" | "thinking";
  text: string;
}

/// Accumulates display-only worker log fragments (`task-log`) into per-task ordered
/// tagged segments, coalescing consecutive same-kind deltas. `logFor` returns the
/// OUTPUT-only flat text (the live-log state machine keys off it); `segmentsFor`
/// returns the interleaved segments for rendering.
export function useTaskLog() {
  const buffers = useRef<Record<string, LogSegment[]>>({});
  const [, setVersion] = useState(0);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const un = await onTaskLog((log) => {
        const segs = buffers.current[log.task_id] ?? (buffers.current[log.task_id] = []);
        const last = segs[segs.length - 1];
        if (last && last.kind === log.kind) last.text += log.delta;
        else segs.push({ kind: log.kind, text: log.delta });
        setVersion((v) => v + 1);
      });
      if (cancelled) un();
      else unlisten = un;
    })();
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  const segmentsFor = useCallback((taskId: string): LogSegment[] => {
    return buffers.current[taskId] ?? [];
  }, []);

  const logFor = useCallback((taskId: string): string => {
    return (buffers.current[taskId] ?? [])
      .filter((s) => s.kind === "output")
      .map((s) => s.text)
      .join("");
  }, []);

  return { logFor, segmentsFor };
}
