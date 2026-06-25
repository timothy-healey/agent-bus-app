import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";

// Capture the registered task-log callback so the test can push fragments.
let logCb: ((log: { task_id: string; delta: string; kind: "output" | "thinking" }) => void) | undefined;
vi.mock("../ipc/runtime", () => ({
  onTaskLog: vi.fn(async (cb: (log: { task_id: string; delta: string; kind: "output" | "thinking" }) => void) => {
    logCb = cb;
    return () => { logCb = undefined; };
  }),
}));

import { useTaskLog } from "./useTaskLog";

describe("useTaskLog", () => {
  beforeEach(() => { logCb = undefined; });

  it("accumulates output fragments per task id (logFor is output-only)", async () => {
    const { result } = renderHook(() => useTaskLog());
    // wait a tick for the async subscribe
    await act(async () => { await Promise.resolve(); });
    act(() => {
      logCb?.({ task_id: "T-1", delta: "chunk-a ", kind: "output" });
      logCb?.({ task_id: "T-1", delta: "chunk-b", kind: "output" });
      logCb?.({ task_id: "T-2", delta: "other", kind: "output" });
    });
    expect(result.current.logFor("T-1")).toBe("chunk-a chunk-b");
    expect(result.current.logFor("T-2")).toBe("other");
    expect(result.current.logFor("T-3")).toBe("");
  });
});

describe("useTaskLog tagged segments", () => {
  beforeEach(() => { logCb = undefined; });

  it("accumulates ordered segments, coalescing consecutive same-kind deltas", async () => {
    const { result } = renderHook(() => useTaskLog());
    await waitFor(() => expect(logCb).not.toBeUndefined());
    act(() => logCb!({ task_id: "T-1", delta: "think a", kind: "thinking" }));
    act(() => logCb!({ task_id: "T-1", delta: " think b", kind: "thinking" }));
    act(() => logCb!({ task_id: "T-1", delta: "out a", kind: "output" }));
    act(() => logCb!({ task_id: "T-1", delta: "think c", kind: "thinking" }));
    await waitFor(() => expect(result.current.segmentsFor("T-1")).toHaveLength(3));
    expect(result.current.segmentsFor("T-1")).toEqual([
      { kind: "thinking", text: "think a think b" },
      { kind: "output", text: "out a" },
      { kind: "thinking", text: "think c" },
    ]);
    // logFor still returns the OUTPUT-only flat text (back-compat for the state machine)
    expect(result.current.logFor("T-1")).toBe("out a");
    expect(result.current.segmentsFor("T-2")).toEqual([]);
  });
});
