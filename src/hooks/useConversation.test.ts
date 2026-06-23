import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";

const getConversationMock = vi.fn();
const sendMessageMock = vi.fn();
const onConversationDeltaMock = vi.fn();
vi.mock("../ipc/terminal", () => ({
  getConversation: () => getConversationMock(),
  sendMessage: (input: string) => sendMessageMock(input),
  onConversationDelta: (cb: (d: { text: string; reset: boolean }) => void) => onConversationDeltaMock(cb),
}));

import { useConversation } from "./useConversation";

function convo(turns: unknown[]) {
  return { project_id: "p", session_id: "s", started_at: 0, last_message_at: 0, turns, summary_of_prior_sessions: null, history_budget_tokens: 8000 };
}

beforeEach(() => {
  getConversationMock.mockReset();
  sendMessageMock.mockReset();
  onConversationDeltaMock.mockReset();
  onConversationDeltaMock.mockResolvedValue(() => {});
});

describe("useConversation", () => {
  it("loads the conversation on mount", async () => {
    getConversationMock.mockResolvedValue(convo([{ role: "user", text: "hi", tool_calls: [], at: 0 }]));
    const { result } = renderHook(() => useConversation());
    await waitFor(() => expect(result.current.turns).toHaveLength(1));
  });

  it("send appends the returned turns", async () => {
    getConversationMock.mockResolvedValue(null);
    sendMessageMock.mockResolvedValue(convo([
      { role: "user", text: "/inject x", tool_calls: [], at: 1 },
      { role: "assistant", text: "Done: inject_topic.", tool_calls: [], at: 1 },
    ]));
    const { result } = renderHook(() => useConversation());
    await act(async () => { await result.current.send("/inject x"); });
    expect(sendMessageMock).toHaveBeenCalledWith("/inject x");
    await waitFor(() => expect(result.current.turns).toHaveLength(2));
  });

  it("accumulates conversation.delta into streaming text and resets on boundary", async () => {
    getConversationMock.mockResolvedValue(null);
    let deltaCb: ((d: { text: string; reset: boolean }) => void) | undefined;
    onConversationDeltaMock.mockImplementation(async (cb: (d: { text: string; reset: boolean }) => void) => {
      deltaCb = cb;
      return () => {};
    });
    const { result } = renderHook(() => useConversation());
    await waitFor(() => expect(deltaCb).toBeDefined());
    await act(async () => { deltaCb?.({ text: "Hel", reset: false }); });
    await act(async () => { deltaCb?.({ text: "lo", reset: false }); });
    expect(result.current.streaming).toBe("Hello");
    await act(async () => { deltaCb?.({ text: "", reset: true }); });
    expect(result.current.streaming).toBe("");
  });
});
