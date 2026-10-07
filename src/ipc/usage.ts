import { invoke } from "@tauri-apps/api/core";

export type ThresholdBand = "safe" | "warn" | "hot" | "braked";

/// One plan Limit as the meter shows it.
export interface LimitView {
  label: string;
  utilization_pct: number;
  resets_in_secs: number;
}

export interface TeamSlice {
  team_id: string;
  tokens: number;
  /// List-price Cost in the current 5-hour window. Informational only.
  cost_usd: number;
}

export interface UsageSnapshot {
  available: boolean;
  observed_at: number | null;
  session: LimitView | null;
  weekly: LimitView | null;
  model_scoped: LimitView[];
  band: ThresholdBand;
  braked: boolean;
  auto_meter_enabled: boolean;
  by_team: TeamSlice[];
  tokens_by_task: Record<string, number>;
  /// Why the last poll failed; null after a successful poll.
  last_error: string | null;
}

export async function usageSnapshot(): Promise<UsageSnapshot> {
  return await invoke<UsageSnapshot>("usage_snapshot");
}

export async function setAutoMeter(enabled: boolean): Promise<UsageSnapshot> {
  return await invoke<UsageSnapshot>("usage_set_auto_meter", { enabled });
}
