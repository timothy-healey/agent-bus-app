import { useCallback, useEffect, useState } from "react";
import { listSkills, type SkillEntry } from "../ipc/skills";

export interface SkillCatalog {
  entries: SkillEntry[];
  loading: boolean;
  /** Re-scan the catalog for the active project (the "refresh skills" action). */
  refresh: () => void;
}

/// A4/G4 — load the skill catalog at boot + on a manual refresh, holding it in
/// memory. `projectId` is OPTIONAL: with a project, the catalog merges the
/// project's sources over the global `~/.claude`; with `null` (the new-project
/// wizard, before a project exists) it loads the GLOBAL `~/.claude` catalog — it
/// does NOT short-circuit to empty, so skills are discoverable during creation
/// (G4). Failures degrade to an empty catalog (autocomplete is sugar; a discovery
/// hiccup must not break prompt authoring).
export function useSkillCatalog(projectId: string | null): SkillCatalog {
  const [entries, setEntries] = useState<SkillEntry[]>([]);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(() => {
    setLoading(true);
    // null projectId → global-only catalog (the backend resolves global roots).
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
