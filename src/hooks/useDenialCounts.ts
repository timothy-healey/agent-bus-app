import { useCallback, useEffect, useState } from "react";
import { denialCounts } from "../ipc/runtime";
import { useRuntimeEvents } from "./useRuntimeEvents";

/// Permission denials per task id, for the card badges. Loads once, then
/// refetches on every `task-changed` (a settled invocation is when denials
/// land). A failed fetch keeps the last counts.
export function useDenialCounts(): Record<string, number> {
  const [counts, setCounts] = useState<Record<string, number>>({});

  const reload = useCallback(() => {
    Promise.resolve()
      .then(() => denialCounts())
      .then((c) => setCounts(c ?? {}))
      .catch(() => {});
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  useRuntimeEvents({ onTaskChanged: reload });

  return counts;
}
