import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { useUsage } from "./useUsage";
import type { UsageSnapshot } from "../ipc/usage";

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

const base = (over: Partial<UsageSnapshot> = {}): UsageSnapshot => ({
  available: true,
  observed_at: 1_000,
  session: { label: "session (5h)", utilization_pct: 41, resets_in_secs: 3_600 },
  weekly: { label: "weekly (7d)", utilization_pct: 2, resets_in_secs: 86_400 },
  model_scoped: [{ label: "Fable weekly", utilization_pct: 0, resets_in_secs: 86_400 }],
  band: "safe",
  braked: false,
  auto_meter_enabled: false,
  by_team: [{ team_id: "research", tokens: 240, cost_usd: 0.25 }],
  tokens_by_task: {},
  ...over,
});
const snap = (pct: number) => base({ session: { label: "session (5h)", utilization_pct: pct * 100, resets_in_secs: 1 } });

describe("useUsage", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    for (const k of Object.keys(listeners)) delete listeners[k];
  });

  it("loads a snapshot on mount", async () => {
    invokeMock.mockResolvedValue(snap(0.5));
    const { result } = renderHook(() => useUsage());
    await waitFor(() => expect(result.current.snapshot?.session?.utilization_pct).toBe(50));
    expect(invokeMock).toHaveBeenCalledWith("usage_snapshot");
  });

  it("refetches when usage.changed fires", async () => {
    invokeMock.mockResolvedValueOnce(snap(0.2)).mockResolvedValueOnce(snap(0.8));
    const { result } = renderHook(() => useUsage());
    await waitFor(() => expect(result.current.snapshot?.session?.utilization_pct).toBe(20));
    await act(async () => { await Promise.resolve(); });
    act(() => listeners["usage-changed"]?.({ payload: null }));
    await waitFor(() => expect(result.current.snapshot?.session?.utilization_pct).toBe(80));
  });
});
