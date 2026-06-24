import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { useUsage } from "./useUsage";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const listeners: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    listeners[name] = cb;
    return () => delete listeners[name];
  }),
}));

const snap = (pct: number) => ({
  window_total: 100, window_budget: 200, window_pct: pct, band: "safe",
  burn_per_min: 0, window_secs: 18000, reset_in_secs: null, est_brake_at: null,
  by_team: [], tokens_by_task: {}, braked: false,
});

describe("useUsage", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    for (const k of Object.keys(listeners)) delete listeners[k];
  });

  it("loads a snapshot on mount", async () => {
    invokeMock.mockResolvedValue(snap(0.5));
    const { result } = renderHook(() => useUsage());
    await waitFor(() => expect(result.current.snapshot?.window_pct).toBe(0.5));
    expect(invokeMock).toHaveBeenCalledWith("usage_snapshot");
  });

  it("refetches when usage.changed fires", async () => {
    invokeMock.mockResolvedValueOnce(snap(0.2)).mockResolvedValueOnce(snap(0.8));
    const { result } = renderHook(() => useUsage());
    await waitFor(() => expect(result.current.snapshot?.window_pct).toBe(0.2));
    await act(async () => { await Promise.resolve(); });
    act(() => listeners["usage-changed"]?.({ payload: null }));
    await waitFor(() => expect(result.current.snapshot?.window_pct).toBe(0.8));
  });
});
