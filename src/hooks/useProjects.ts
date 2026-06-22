import { useEffect, useState } from "react";
import { listProjects, type Project } from "../ipc/workspace";

export function useProjects(): { projects: Project[]; loading: boolean; reload: () => void } {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    listProjects()
      .then((list) => { if (!cancelled) setProjects(list); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [tick]);

  return { projects, loading, reload: () => setTick((t) => t + 1) };
}
