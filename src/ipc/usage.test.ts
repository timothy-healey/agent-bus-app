import { describe, expect, it, vi, beforeEach } from "vitest";
import { usageSnapshot, setBudget, setAutoMeter } from "./usage";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe("usage ipc", () => {
  beforeEach(() => invokeMock.mockReset());

  it("usageSnapshot calls usage_snapshot", async () => {
    invokeMock.mockResolvedValueOnce({ window_total: 0, window_budget: 1, window_pct: 0, band: "safe", burn_per_min: 0, window_secs: 18000, reset_in_secs: null, est_brake_at: null, by_team: [], tokens_by_task: {}, braked: false });
    const s = await usageSnapshot();
    expect(invokeMock).toHaveBeenCalledWith("usage_snapshot");
    expect(s.band).toBe("safe");
  });

  it("setBudget passes the budget", async () => {
    invokeMock.mockResolvedValueOnce({ window_budget: 5000000 });
    await setBudget(5_000_000);
    expect(invokeMock).toHaveBeenCalledWith("usage_set_budget", { budget: 5_000_000 });
  });

  it("setAutoMeter passes the enabled flag", async () => {
    invokeMock.mockResolvedValueOnce({ auto_meter_enabled: true });
    await setAutoMeter(true);
    expect(invokeMock).toHaveBeenCalledWith("usage_set_auto_meter", { enabled: true });
  });
});
