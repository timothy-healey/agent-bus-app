import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { usageSnapshot, type UsageSnapshot } from "../ipc/usage";
import { EVENTS } from "../ipc/events";

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
      const a = await listen(EVENTS.usageChanged, () => reload());
      const b = await listen(EVENTS.taskChanged, () => reload());
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
