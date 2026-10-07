import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";

const denialCountsMock = vi.fn();
let capturedOnTaskChanged: ((id: string) => void) | undefined;

vi.mock("../ipc/runtime", () => ({
  denialCounts: () => denialCountsMock(),
}));

vi.mock("./useRuntimeEvents", () => ({
  useRuntimeEvents: ({ onTaskChanged }: { onTaskChanged?: (id: string) => void }) => {
    capturedOnTaskChanged = onTaskChanged;
  },
}));

import { useDenialCounts } from "./useDenialCounts";

describe("useDenialCounts", () => {
  beforeEach(() => {
    denialCountsMock.mockReset();
    capturedOnTaskChanged = undefined;
  });

  it("loads the counts on mount and refetches on task-changed", async () => {
    denialCountsMock.mockResolvedValueOnce({ "T-1": 1 });
    const { result } = renderHook(() => useDenialCounts());
    await waitFor(() => expect(result.current).toEqual({ "T-1": 1 }));
    denialCountsMock.mockResolvedValueOnce({ "T-1": 1, "T-2": 4 });
    act(() => capturedOnTaskChanged?.("T-2"));
    await waitFor(() => expect(result.current).toEqual({ "T-1": 1, "T-2": 4 }));
  });

  it("keeps the last counts when a fetch fails", async () => {
    denialCountsMock.mockResolvedValueOnce({ "T-1": 1 });
    const { result } = renderHook(() => useDenialCounts());
    await waitFor(() => expect(result.current).toEqual({ "T-1": 1 }));
    denialCountsMock.mockRejectedValueOnce(new Error("no backend"));
    act(() => capturedOnTaskChanged?.("T-1"));
    await new Promise((r) => setTimeout(r, 10));
    expect(result.current).toEqual({ "T-1": 1 });
  });
});
