import { useCallback, useEffect, useState } from "react";
import { getConversation, sendMessage, type Turn } from "../ipc/terminal";

/// Loads the project's persistent conversation and exposes a `send` that posts a
/// user line and replaces local turns with the backend's authoritative result
/// (the backend appended user + assistant turns and persisted them).
export function useConversation() {
  const [turns, setTurns] = useState<Turn[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    getConversation()
      .then((c) => { if (!cancelled && c) setTurns(c.turns); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, []);

  const send = useCallback(async (input: string) => {
    const trimmed = input.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      const c = await sendMessage(trimmed);
      setTurns(c.turns);
    } finally {
      setBusy(false);
    }
  }, []);

  return { turns, send, busy };
}
