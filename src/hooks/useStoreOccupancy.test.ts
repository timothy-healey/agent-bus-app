import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import type { StoreOccupancy } from "../ipc/runtime";

const runStoreOccupancyMock = vi.fn();
let capturedOnTaskChanged: ((id: string) => void) | undefined;

vi.mock("../ipc/runtime", () => ({
  runStoreOccupancy: (id: string) => runStoreOccupancyMock(id),
}));

vi.mock("./useRuntimeEvents", () => ({
  useRuntimeEvents: ({ onTaskChanged }: { onTaskChanged?: (id: string) => void }) => {
    capturedOnTaskChanged = onTaskChanged;
  },
}));

import { useStoreOccupancy } from "./useStoreOccupancy";

const occ = (occupancy: number): StoreOccupancy[] => [{ stage: "spec", occupancy, capacity: 3 }];

describe("useStoreOccupancy", () => {
  beforeEach(() => {
    runStoreOccupancyMock.mockReset();
    capturedOnTaskChanged = undefined;
  });

  it("loads occupancy for the run on mount", async () => {
    runStoreOccupancyMock.mockResolvedValueOnce(occ(1));
    const { result } = renderHook(() => useStoreOccupancy("R-1"));
    await waitFor(() => expect(result.current.occupancy[0]?.occupancy).toBe(1));
    expect(runStoreOccupancyMock).toHaveBeenCalledWith("R-1");
  });

  it("refetches when a task-changed event arrives", async () => {
    runStoreOccupancyMock.mockResolvedValueOnce(occ(1));
    const { result } = renderHook(() => useStoreOccupancy("R-1"));
    await waitFor(() => expect(result.current.occupancy[0]?.occupancy).toBe(1));
    runStoreOccupancyMock.mockResolvedValueOnce(occ(2));
    act(() => capturedOnTaskChanged?.("T-2"));
    await waitFor(() => expect(result.current.occupancy[0]?.occupancy).toBe(2));
  });

  it("clears occupancy and does not call IPC for a null run", async () => {
    const { result } = renderHook(() => useStoreOccupancy(null));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.occupancy).toEqual([]);
    expect(runStoreOccupancyMock).not.toHaveBeenCalled();
  });
});
