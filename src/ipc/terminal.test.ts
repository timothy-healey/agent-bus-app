import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

const listenMock = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listenMock(...a) }));

import { sendMessage, getConversation, onConversationDelta } from "./terminal";

beforeEach(() => { invokeMock.mockReset(); listenMock.mockReset(); });

describe("terminal ipc", () => {
  it("sendMessage invokes send_message with the input arg", async () => {
    invokeMock.mockResolvedValue({ project_id: "p", session_id: "s", started_at: 0, last_message_at: 1, turns: [], summary_of_prior_sessions: null, history_budget_tokens: 8000 });
    await sendMessage("/inject 03-x");
    expect(invokeMock).toHaveBeenCalledWith("send_message", { input: "/inject 03-x" });
  });

  it("getConversation invokes get_conversation with no args", async () => {
    invokeMock.mockResolvedValue(null);
    const c = await getConversation();
    expect(invokeMock).toHaveBeenCalledWith("get_conversation");
    expect(c).toBeNull();
  });

  it("onConversationDelta subscribes to conversation.delta and forwards payloads", async () => {
    const handlers: Array<(e: { payload: unknown }) => void> = [];
    listenMock.mockImplementation((_name: string, cb: (e: { payload: unknown }) => void) => {
      handlers.push(cb);
      return Promise.resolve(() => {});
    });
    const got: Array<{ text: string; reset: boolean }> = [];
    await onConversationDelta((d) => got.push(d));
    expect(listenMock).toHaveBeenCalledWith("conversation.delta", expect.any(Function));
    handlers[0]({ payload: { text: "hi", reset: false } });
    expect(got).toEqual([{ text: "hi", reset: false }]);
  });
});
