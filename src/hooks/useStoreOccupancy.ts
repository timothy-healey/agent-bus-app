import { useCallback, useEffect, useState } from "react";
import { runStoreOccupancy, type StoreOccupancy } from "../ipc/runtime";
import { useRuntimeEvents } from "./useRuntimeEvents";

export interface UseStoreOccupancy {
  occupancy: StoreOccupancy[];
  loading: boolean;
  reload: () => void;
}

/// Per-stage store occupancy for the selected run (④e board lane indicators).
/// Loads on run change and refetches on every `task-changed` event — occupancy
/// moves as workers reserve/commit/release slots, which is exactly what
/// task-changed signals. `null` runId clears the data (no run scoped).
export function useStoreOccupancy(runId: string | null): UseStoreOccupancy {
  const [occupancy, setOccupancy] = useState<StoreOccupancy[]>([]);
  const [loading, setLoading] = useState(runId != null);

  const reload = useCallback(() => {
    if (runId == null) {
      setOccupancy([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    runStoreOccupancy(runId)
      .then(setOccupancy)
      .catch(() => setOccupancy([]))
      .finally(() => setLoading(false));
  }, [runId]);

  useEffect(() => {
    reload();
  }, [reload]);

  useRuntimeEvents({ onTaskChanged: reload });

  return { occupancy, loading, reload };
}
