import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";

const listTasksMock = vi.fn();
let capturedOnTaskChanged: ((id: string) => void) | undefined;

vi.mock("../ipc/runtime", () => ({
  listTasks: () => listTasksMock(),
}));

vi.mock("./useRuntimeEvents", () => ({
  useRuntimeEvents: ({ onTaskChanged }: { onTaskChanged?: (id: string) => void }) => {
    capturedOnTaskChanged = onTaskChanged;
  },
}));

import { useTasks } from "./useTasks";

describe("useTasks", () => {
  beforeEach(() => {
    listTasksMock.mockReset();
    capturedOnTaskChanged = undefined;
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
});
