import { useCallback, useEffect, useState } from "react";
import { listSkills, type SkillEntry } from "../ipc/skills";

export interface SkillCatalog {
  entries: SkillEntry[];
  loading: boolean;
  /** Re-scan the catalog for the active project (the "refresh skills" action). */
  refresh: () => void;
}

/// A4 — load the skill catalog for the active project at boot + on a manual
/// refresh, holding it in memory. `projectId` null → an empty, idle catalog
/// (no project selected yet). Failures degrade to an empty catalog (autocomplete
/// is sugar; a discovery hiccup must not break prompt authoring).
export function useSkillCatalog(projectId: string | null): SkillCatalog {
  const [entries, setEntries] = useState<SkillEntry[]>([]);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(() => {
    if (!projectId) {
      setEntries([]);
      return;
    }
    setLoading(true);
    listSkills(projectId)
      .then(setEntries)
      .catch(() => setEntries([]))
      .finally(() => setLoading(false));
  }, [projectId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { entries, loading, refresh };
}
