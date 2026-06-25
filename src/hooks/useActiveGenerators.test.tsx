import { describe, it, expect, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { vi } from "vitest";

// Capture the registered callback so the test can push events.
let cb: ((s: { run_id: string; stage: string; task_id: string; active: boolean }) => void) | null = null;
vi.mock("../ipc/runtime", () => ({
  onGeneratorStatus: (fn: typeof cb) => {
    cb = fn;
    return Promise.resolve(() => { cb = null; });
  },
}));

import { useActiveGenerators } from "./useActiveGenerators";

describe("useActiveGenerators", () => {
  beforeEach(() => { cb = null; });

  it("adds an active generator and clears it on active:false, scoped to the run", async () => {
    const { result } = renderHook(() => useActiveGenerators("R-1"));
    await waitFor(() => expect(cb).not.toBeNull());
    act(() => cb!({ run_id: "R-1", stage: "research", task_id: "gen:R-1:research", active: true }));
    await waitFor(() => expect(result.current).toHaveLength(1));
    expect(result.current[0]).toMatchObject({ stage: "research", task_id: "gen:R-1:research" });
    // an event for a different run is ignored
    act(() => cb!({ run_id: "R-2", stage: "research", task_id: "gen:R-2:research", active: true }));
    expect(result.current).toHaveLength(1);
    // active:false clears it
    act(() => cb!({ run_id: "R-1", stage: "research", task_id: "gen:R-1:research", active: false }));
    await waitFor(() => expect(result.current).toHaveLength(0));
  });

  it("returns nothing when no run is scoped", () => {
    const { result } = renderHook(() => useActiveGenerators(null));
    expect(result.current).toEqual([]);
  });
});
