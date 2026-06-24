import { useCallback, useEffect, useMemo, useState } from "react";
import { listRuns, type Run } from "../ipc/runtime";
import { useRuntimeEvents } from "./useRuntimeEvents";

export interface UseRuns {
  runs: Run[];
  loading: boolean;
  /// The run the board is currently scoped to (the selected run, falling back to
  /// the active run, then the newest run). `null` until runs load / none exist.
  selectedRun: Run | null;
  /// The active run: the newest not-`completed` run for the project (what a fresh
  /// Start lands on). `null` when every run is completed / none exist.
  activeRun: Run | null;
  /// Operator override of which run the board scopes to. Passing an id not in the
  /// list (or a stale one) harmlessly falls back to the active/newest run.
  select: (runId: string) => void;
  reload: () => void;
}

/// Run selector data source (④e): loads the project's runs (newest first), then
/// refetches on every `run-changed` event. Tracks the selected run; defaults to
/// the active (newest incomplete) run, falling back to the newest run. Reselects
/// the freshest run when a brand-new run appears and the operator hasn't pinned a
/// selection — so Start lands you on the new run automatically.
export function useRuns(projectId: string | null): UseRuns {
  const [runs, setRuns] = useState<Run[]>([]);
  const [loading, setLoading] = useState(projectId != null);
  // null = follow the active/newest run; a string pins the operator's choice.
  const [pinnedId, setPinnedId] = useState<string | null>(null);

  const reload = useCallback(() => {
    if (projectId == null) {
      setRuns([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    listRuns(projectId)
      .then(setRuns)
      .catch(() => setRuns([]))
      .finally(() => setLoading(false));
  }, [projectId]);

  useEffect(() => {
    // a project switch clears any pinned selection (a pin is per-project).
    setPinnedId(null);
    reload();
  }, [reload]);

  useRuntimeEvents({ onRunChanged: reload });

  // runs are newest-first; the active run is the newest not-completed one.
  const activeRun = useMemo(() => runs.find((r) => !r.completed) ?? null, [runs]);

  const selectedRun = useMemo(() => {
    if (pinnedId != null) {
      const pinned = runs.find((r) => r.id === pinnedId);
      if (pinned) return pinned;
    }
    // unpinned (or stale pin): follow the active run, then the newest run.
    return activeRun ?? runs[0] ?? null;
  }, [pinnedId, runs, activeRun]);

  const select = useCallback((runId: string) => setPinnedId(runId), []);

  return { runs, loading, selectedRun, activeRun, select, reload };
}
