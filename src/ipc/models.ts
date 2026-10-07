import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { EVENTS } from "./events";

/// The models the installed Claude CLI offers, each with the effort levels it
/// supports. The one source of truth for which Model and Effort combinations
/// are valid; the backend reads it from the CLI for free (no model call).

export type ModelListSource = "live" | "cached" | "curated";

export interface ModelOption {
  /// What is saved as the team's model and passed as `--model`.
  value: string;
  resolved_model: string | null;
  display_name: string;
  description: string | null;
  /// Empty when the model takes no effort level (only Default).
  effort_levels: string[];
  /// Whether workers on this model run in auto mode (needed for Remote git).
  supports_auto_mode: boolean;
}

export interface ModelList {
  models: ModelOption[];
  source: ModelListSource;
  cli_version: string | null;
}

/// The model new teams use: the CLI's `default` alias. Mirrors
/// `DEFAULT_TEAM_MODEL` in `agent_bus_core`.
export const DEFAULT_TEAM_MODEL = "default";

export async function getModelList(): Promise<ModelList> {
  return await invoke<ModelList>("model_list");
}

/// Re-query the CLI. Resolves to the list now current: the fresh one, or the
/// previous one if the query failed.
export async function refreshModelList(): Promise<ModelList> {
  return await invoke<ModelList>("refresh_model_list");
}

/// Call `cb` with the new list whenever a live fetch replaces it. Resolves to
/// the unlisten function.
export async function onModelListUpdated(cb: (list: ModelList) => void): Promise<() => void> {
  return await listen(EVENTS.modelListUpdated, async () => cb(await getModelList()));
}

export function findModel(list: ModelList | null, value: string): ModelOption | undefined {
  return list?.models.find((m) => m.value === value);
}

/// An alias shows what it resolves to ("opus → claude-opus-5-5"); a pinned id
/// shows itself.
export function modelLabel(m: ModelOption): string {
  return m.resolved_model && m.resolved_model !== m.value ? `${m.value} → ${m.resolved_model}` : m.value;
}

export function sourceLabel(s: ModelListSource): string {
  return s === "curated" ? "built-in" : s;
}

export function supportsEffort(list: ModelList | null, model: string, level: string): boolean {
  return findModel(list, model)?.effort_levels.includes(level) ?? false;
}
