import { useEffect, useState } from "react";
import { onGeneratorStatus, type GeneratorStatus } from "../ipc/runtime";

/// One currently-active generator pass — the transient source card the board shows.
export interface ActiveGenerator {
  stage: string;
  task_id: string;
}

/// Accumulates the generators currently mid-pass for `runId` (set on `active:true`,
/// cleared on `active:false`). Events for other runs are ignored. Returns `[]` when
/// no run is scoped. Keyed by `task_id` (the stable `gen:<run>:<stage>` id).
export function useActiveGenerators(runId: string | null): ActiveGenerator[] {
  const [active, setActive] = useState<Record<string, ActiveGenerator>>({});

  useEffect(() => {
    // A run switch resets the set so a stale generator never lingers.
    setActive({});
    if (!runId) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const un = await onGeneratorStatus((s: GeneratorStatus) => {
        if (s.run_id !== runId) return;
        setActive((prev) => {
          const next = { ...prev };
          if (s.active) next[s.task_id] = { stage: s.stage, task_id: s.task_id };
          else delete next[s.task_id];
          return next;
        });
      });
      if (cancelled) un();
      else unlisten = un;
    })();
    return () => { cancelled = true; unlisten?.(); };
  }, [runId]);

  return Object.values(active);
}
