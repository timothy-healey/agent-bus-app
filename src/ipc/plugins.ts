import { invoke } from "@tauri-apps/api/core";

/// A Claude Code plugin a team can declare: installed, or offered by a local
/// directory marketplace. Mirrors the Rust `workspace::plugins::PluginInfo`.
export interface PluginInfo {
  name: string;
  marketplace: string;
  path: string;
}

/// The plugins a team can declare, read from the Claude config root.
export async function listPlugins(): Promise<PluginInfo[]> {
  return await invoke<PluginInfo[]>("list_plugins");
}

/// How a plugin reads in the editor: its name and marketplace.
export function pluginLabel(p: PluginInfo): string {
  return `${p.name} (${p.marketplace})`;
}
