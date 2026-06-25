import { useCallback, useEffect, useState } from "react";
import { getConversation, onConversationDelta, sendMessage, type Turn } from "../ipc/terminal";

/// Loads the project's persistent conversation and exposes a `send` that posts a
/// user line and replaces local turns with the backend's authoritative result
/// (the backend appended user + assistant turns and persisted them). Also
/// subscribes to display-only `conversation.delta` streaming fragments,
/// accumulating them into `streaming` (a transient assistant bubble) that the
/// terminal shows until the authoritative turns arrive.
export function useConversation() {
  const [turns, setTurns] = useState<Turn[]>([]);
  const [busy, setBusy] = useState(false);
  const [streaming, setStreaming] = useState("");

  useEffect(() => {
    let cancelled = false;
    getConversation()
      .then((c) => { if (!cancelled && c) setTurns(c.turns); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const un = await onConversationDelta((d) => {
        setStreaming((prev) => (d.reset ? "" : prev + d.text));
      });
      if (cancelled) un();
      else unlisten = un;
    })();
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  const send = useCallback(async (input: string) => {
    const trimmed = input.trim();
    if (!trimmed) return;
    // Optimistically echo the user's line so the chat registers INSTANTLY (the
    // backend turn can take a while — claude thinking). Replaced by the
    // authoritative turns (which re-include this user turn + the assistant reply)
    // when sendMessage resolves.
    setTurns((prev) => [...prev, { role: "user", text: trimmed, tool_calls: [], at: Date.now() }]);
    setBusy(true);
    try {
      const c = await sendMessage(trimmed);
      setTurns(c.turns);
    } finally {
      setStreaming("");
      setBusy(false);
    }
  }, []);

  return { turns, send, busy, streaming };
}
