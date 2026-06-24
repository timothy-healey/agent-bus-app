import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import type { Run } from "../ipc/runtime";

const listRunsMock = vi.fn();
let capturedOnRunChanged: ((id: string) => void) | undefined;

vi.mock("../ipc/runtime", () => ({
  listRuns: (id: string) => listRunsMock(id),
}));

vi.mock("./useRuntimeEvents", () => ({
  useRuntimeEvents: ({ onRunChanged }: { onRunChanged?: (id: string) => void }) => {
    capturedOnRunChanged = onRunChanged;
  },
}));

import { useRuns } from "./useRuns";

const run = (id: string, completed = false, createdAt = 0): Run => ({
  id, pipeline: "p", project_id: "proj", generator_dry: false, completed, created_at: createdAt,
});

describe("useRuns", () => {
  beforeEach(() => {
    listRunsMock.mockReset();
    capturedOnRunChanged = undefined;
  });

  it("loads the project's runs and defaults the selection to the active (newest incomplete) run", async () => {
    // newest-first: R-2 (incomplete) is active; R-1 completed.
    listRunsMock.mockResolvedValueOnce([run("R-2", false, 200), run("R-1", true, 100)]);
    const { result } = renderHook(() => useRuns("proj"));
    await waitFor(() => expect(result.current.runs).toHaveLength(2));
    expect(result.current.activeRun?.id).toBe("R-2");
    expect(result.current.selectedRun?.id).toBe("R-2");
  });

  it("falls back to the newest run when every run is completed", async () => {
    listRunsMock.mockResolvedValueOnce([run("R-2", true, 200), run("R-1", true, 100)]);
    const { result } = renderHook(() => useRuns("proj"));
    await waitFor(() => expect(result.current.runs).toHaveLength(2));
    expect(result.current.activeRun).toBeNull();
    expect(result.current.selectedRun?.id).toBe("R-2");
  });

  it("honors an explicit selection and keeps it pinned", async () => {
    listRunsMock.mockResolvedValueOnce([run("R-2", false, 200), run("R-1", true, 100)]);
    const { result } = renderHook(() => useRuns("proj"));
    await waitFor(() => expect(result.current.runs).toHaveLength(2));
    act(() => result.current.select("R-1"));
    await waitFor(() => expect(result.current.selectedRun?.id).toBe("R-1"));
  });

  it("refetches when a run-changed event arrives", async () => {
    listRunsMock.mockResolvedValueOnce([run("R-1", false, 100)]);
    const { result } = renderHook(() => useRuns("proj"));
    await waitFor(() => expect(result.current.runs).toHaveLength(1));
    // a new run appears; unpinned selection follows the freshest active run.
    listRunsMock.mockResolvedValueOnce([run("R-2", false, 200), run("R-1", false, 100)]);
    act(() => capturedOnRunChanged?.("R-2"));
    await waitFor(() => expect(result.current.selectedRun?.id).toBe("R-2"));
  });

  it("returns no runs and does not call IPC when the project is null", async () => {
    const { result } = renderHook(() => useRuns(null));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.runs).toEqual([]);
    expect(result.current.selectedRun).toBeNull();
    expect(listRunsMock).not.toHaveBeenCalled();
  });
});
