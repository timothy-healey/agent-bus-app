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

  it("calls onRunChanged when a run-changed event fires", async () => {
    const onRunChanged = vi.fn();
    renderHook(() => useRuntimeEvents({ onRunChanged }));
    await act(async () => { await Promise.resolve(); });
    act(() => listeners["run-changed"]?.({ payload: "R-7" }));
    expect(onRunChanged).toHaveBeenCalledWith("R-7");
  });

  it("does not raise an unhandled rejection when unlisten throws on cleanup", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    // Mimic Tauri's async unlisten that rejects for an already-gone eventId.
    vi.mocked(listen).mockImplementationOnce(async (name, cb) => {
      listeners[name as string] = cb as (e: { payload: unknown }) => void;
      return (async () => {
        throw new TypeError("undefined is not an object (evaluating 'listeners[eventId].handlerId')");
      }) as unknown as ReturnType<typeof listen> extends Promise<infer U> ? U : never;
    });
    const onTaskChanged = vi.fn();
    const { unmount } = renderHook(() => useRuntimeEvents({ onTaskChanged }));
    await act(async () => { await Promise.resolve(); });
    // Cleanup fires the rejecting unlisten; safeUnlisten must swallow it.
    expect(() => unmount()).not.toThrow();
    await act(async () => { await Promise.resolve(); });
  });
});
