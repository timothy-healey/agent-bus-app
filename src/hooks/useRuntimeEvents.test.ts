import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useRuntimeEvents } from "./useRuntimeEvents";

const listeners: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    listeners[name] = cb;
    return () => delete listeners[name];
  }),
}));

describe("useRuntimeEvents", () => {
  beforeEach(() => {
    for (const k of Object.keys(listeners)) delete listeners[k];
  });

  it("calls onTaskChanged when a task.changed event fires", async () => {
    const onTaskChanged = vi.fn();
    renderHook(() => useRuntimeEvents({ onTaskChanged }));
    // let the async listen() resolve
    await act(async () => { await Promise.resolve(); });
    act(() => listeners["task-changed"]?.({ payload: "T-42" }));
    expect(onTaskChanged).toHaveBeenCalledWith("T-42");
  });
});
