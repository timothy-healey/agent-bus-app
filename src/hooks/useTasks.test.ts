import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";

const listTasksMock = vi.fn();
let capturedOnTaskChanged: ((id: string) => void) | undefined;
let capturedOnRunChanged: ((id: string) => void) | undefined;

vi.mock("../ipc/runtime", () => ({
  listTasks: () => listTasksMock(),
}));

vi.mock("./useRuntimeEvents", () => ({
  useRuntimeEvents: ({ onTaskChanged, onRunChanged }: { onTaskChanged?: (id: string) => void; onRunChanged?: (id: string) => void }) => {
    capturedOnTaskChanged = onTaskChanged;
    capturedOnRunChanged = onRunChanged;
  },
}));

import { useTasks } from "./useTasks";

describe("useTasks", () => {
  beforeEach(() => {
    listTasksMock.mockReset();
    capturedOnTaskChanged = undefined;
    capturedOnRunChanged = undefined;
  });

  it("loads tasks on mount", async () => {
    listTasksMock.mockResolvedValueOnce([{ id: "T-1" }]);
    const { result } = renderHook(() => useTasks());
    await waitFor(() => expect(result.current.tasks).toHaveLength(1));
    expect(result.current.tasks[0].id).toBe("T-1");
  });

  it("refetches when a task.changed event arrives", async () => {
    listTasksMock.mockResolvedValueOnce([{ id: "T-1" }]);
    const { result } = renderHook(() => useTasks());
    await waitFor(() => expect(result.current.tasks).toHaveLength(1));

    listTasksMock.mockResolvedValueOnce([{ id: "T-1" }, { id: "T-2" }]);
    act(() => capturedOnTaskChanged?.("T-2"));
    await waitFor(() => expect(result.current.tasks).toHaveLength(2));
  });

  it("refetches when a run-changed event arrives", async () => {
    listTasksMock.mockResolvedValueOnce([{ id: "T-1" }]);
    const { result } = renderHook(() => useTasks());
    await waitFor(() => expect(result.current.tasks).toHaveLength(1));

    listTasksMock.mockResolvedValueOnce([{ id: "T-1" }, { id: "T-2" }]);
    act(() => capturedOnRunChanged?.("R-1"));
    await waitFor(() => expect(result.current.tasks).toHaveLength(2));
  });

  it("groups tasks by run_id (legacy run-less tasks under \"\")", async () => {
    listTasksMock.mockResolvedValueOnce([
      { id: "T-1", run_id: "R-1" },
      { id: "T-2", run_id: "R-1" },
      { id: "T-3", run_id: "R-2" },
      { id: "T-9", run_id: null },
    ]);
    const { result } = renderHook(() => useTasks());
    await waitFor(() => expect(result.current.tasks).toHaveLength(4));
    expect(result.current.tasksByRun.get("R-1")?.map((t) => t.id)).toEqual(["T-1", "T-2"]);
    expect(result.current.tasksByRun.get("R-2")?.map((t) => t.id)).toEqual(["T-3"]);
    expect(result.current.tasksByRun.get("")?.map((t) => t.id)).toEqual(["T-9"]);
  });
});
