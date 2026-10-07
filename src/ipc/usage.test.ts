import { describe, expect, it, vi, beforeEach } from "vitest";
import { usageSnapshot, setAutoMeter } from "./usage";
import type { UsageSnapshot } from "./usage";


const snap = (over: Partial<UsageSnapshot> = {}): UsageSnapshot => ({
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

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe("usage ipc", () => {
  beforeEach(() => invokeMock.mockReset());

  it("usageSnapshot calls usage_snapshot", async () => {
    invokeMock.mockResolvedValueOnce(snap());
    const s = await usageSnapshot();
    expect(invokeMock).toHaveBeenCalledWith("usage_snapshot");
    expect(s.band).toBe("safe");
  });

  it("setAutoMeter passes the enabled flag", async () => {
    invokeMock.mockResolvedValueOnce({ auto_meter_enabled: true });
    await setAutoMeter(true);
    expect(invokeMock).toHaveBeenCalledWith("usage_set_auto_meter", { enabled: true });
  });
});
