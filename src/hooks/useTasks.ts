import { useCallback, useEffect, useMemo, useState } from "react";
import { listTasks, type Task } from "../ipc/runtime";
import { useRuntimeEvents } from "./useRuntimeEvents";

export interface UseTasks {
  tasks: Task[];
  loading: boolean;
  reload: () => void;
  /// Tasks grouped by `run_id` (④e). Legacy tasks with no run id are keyed under
  /// `""` so the board can still surface them. The board reads the entry for the
  /// selected run.
  tasksByRun: Map<string, Task[]>;
}

/// Board data source: loads all tasks once, then refetches the full list on every
/// `task-changed` AND `run-changed` event (④e — a run start seeds work-items whose
/// first appearance rides the run-changed/task-changed pair; v1's coarse refresh).
export function useTasks(): UseTasks {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [loading, setLoading] = useState(true);

  const reload = useCallback(() => {
    setLoading(true);
    listTasks()
      .then((t) => setTasks(t))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  useRuntimeEvents({ onTaskChanged: reload, onRunChanged: reload });

  const tasksByRun = useMemo(() => {
    const m = new Map<string, Task[]>();
    for (const t of tasks) {
      const key = t.run_id ?? "";
      const list = m.get(key);
      if (list) list.push(t);
      else m.set(key, [t]);
    }
    return m;
  }, [tasks]);

  return { tasks, loading, reload, tasksByRun };
}
