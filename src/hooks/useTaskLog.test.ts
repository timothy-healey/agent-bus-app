import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";

// Capture the registered task.log callback so the test can push fragments.
let logCb: ((log: { task_id: string; delta: string }) => void) | undefined;
vi.mock("../ipc/runtime", () => ({
  onTaskLog: vi.fn(async (cb: (log: { task_id: string; delta: string }) => void) => {
    logCb = cb;
    return () => { logCb = undefined; };
  }),
}));

import { useTaskLog } from "./useTaskLog";

describe("useTaskLog", () => {
  beforeEach(() => { logCb = undefined; });

  it("accumulates fragments per task id", async () => {
    const { result } = renderHook(() => useTaskLog());
    // wait a tick for the async subscribe
    await act(async () => { await Promise.resolve(); });
    act(() => {
      logCb?.({ task_id: "T-1", delta: "chunk-a " });
      logCb?.({ task_id: "T-1", delta: "chunk-b" });
      logCb?.({ task_id: "T-2", delta: "other" });
    });
    expect(result.current.logFor("T-1")).toBe("chunk-a chunk-b");
    expect(result.current.logFor("T-2")).toBe("other");
    expect(result.current.logFor("T-3")).toBe("");
  });
});
