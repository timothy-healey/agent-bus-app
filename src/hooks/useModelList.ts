import { useCallback, useEffect, useState } from "react";
import { getModelList, onModelListUpdated, refreshModelList, type ModelList } from "../ipc/models";

/// Swallow an unlisten that throws because the listener is already gone (the
/// StrictMode subscribe→teardown race; see useRuntimeEvents).
function safeUnlisten(u: () => void): void {
  try {
    const r = u() as unknown;
    if (r instanceof Promise) r.catch(() => {});
  } catch {
    /* listener already gone */
  }
}

/// The current model list, kept fresh by `model-list-updated`. `refresh`
/// re-queries the CLI; `list` is null until the first read resolves.
export function useModelList(): { list: ModelList | null; refreshing: boolean; refresh: () => Promise<void> } {
  const [list, setList] = useState<ModelList | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    getModelList()
      .then((l) => {
        if (!cancelled) setList(l);
      })
      .catch(() => {});
    onModelListUpdated((l) => {
      if (!cancelled) setList(l);
    })
      .then((u) => {
        if (cancelled) safeUnlisten(u);
        else unlisten = u;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      if (unlisten) safeUnlisten(unlisten);
    };
  }, []);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      setList(await refreshModelList());
    } catch {
      /* keep the current list; its source label already says what it is */
    } finally {
      setRefreshing(false);
    }
  }, []);

  return { list, refreshing, refresh };
}
