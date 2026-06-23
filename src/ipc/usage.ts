import { invoke } from "@tauri-apps/api/core";

export type ThresholdBand = "safe" | "warn" | "hot" | "braked";

export interface TeamSlice {
  team_id: string;
  tokens: number;
}

export interface UsageSnapshot {
  window_total: number;
  window_budget: number;
  window_pct: number;
  band: ThresholdBand;
  burn_per_min: number;
  window_secs: number;
  reset_in_secs: number | null;
  est_brake_at: number | null;
  by_team: TeamSlice[];
  tokens_by_task: Record<string, number>;
  braked: boolean;
  auto_meter_enabled: boolean;
}

export async function usageSnapshot(): Promise<UsageSnapshot> {
  return await invoke<UsageSnapshot>("usage_snapshot");
}

export async function setBudget(budget: number): Promise<UsageSnapshot> {
  return await invoke<UsageSnapshot>("usage_set_budget", { budget });
}

export async function setAutoMeter(enabled: boolean): Promise<UsageSnapshot> {
  return await invoke<UsageSnapshot>("usage_set_auto_meter", { enabled });
}
