import { useCallback, useEffect, useState } from "react";
import { listTasks, type Task } from "../ipc/runtime";
import { useRuntimeEvents } from "./useRuntimeEvents";

export interface UseTasks {
  tasks: Task[];
  loading: boolean;
  reload: () => void;
}

/// Board data source: loads all tasks once, then refetches the full list on
/// every `task.changed` event (v1's coarse refresh — Plan 3 D9).
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

  useRuntimeEvents({ onTaskChanged: reload });

  return { tasks, loading, reload };
}
