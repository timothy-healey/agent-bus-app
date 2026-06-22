import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { usageSnapshot, type UsageSnapshot } from "../ipc/usage";

export function useUsage(): { snapshot: UsageSnapshot | null; reload: () => void } {
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null);

  const reload = useCallback(() => {
    usageSnapshot()
      .then(setSnapshot)
      .catch(() => setSnapshot(null));
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  useEffect(() => {
    let unlistens: Array<() => void> = [];
    let cancelled = false;
    (async () => {
      const a = await listen("usage.changed", () => reload());
      const b = await listen("task.changed", () => reload());
      if (cancelled) {
        a();
        b();
      } else {
        unlistens = [a, b];
      }
    })();
    return () => {
      cancelled = true;
      unlistens.forEach((u) => u());
    };
  }, [reload]);

  return { snapshot, reload };
}
