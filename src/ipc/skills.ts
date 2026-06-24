import { invoke } from "@tauri-apps/api/core";

/// A4 — skill/command autocomplete. The IPC types here mirror the Rust
/// `SkillEntry` DTO (pinned by a Rust wire-contract test). The frontend never
/// sends a SkillEntry back; this is a read-only discovery surface.

export type SkillKind = "skill" | "command";
export type SkillSource = "global" | "project";

export interface SkillEntry {
  /** Bare invocation name, e.g. "ddd-council", "init-session". */
  name: string;
  kind: SkillKind;
  /** Plugin name for the "plugin:skill" qualified form; null for direct skills. */
  namespace: string | null;
  description: string;
  /** Best-effort declared verbs; [] when none recognized. */
  verbs: string[];
  source: SkillSource;
  /** True when the entry must be inserted under its qualified namespace:name form
   *  (ambiguous name, or lost a cross-source collision to a project entry). */
  qualified: boolean;
}

/** List the skills + slash commands available to a project's worker, for
 *  authoring-time autocomplete. Roots = global ~/.claude + the project's
 *  configured sources (resolved + merged backend-side; project wins). */
export async function listSkills(projectId: string): Promise<SkillEntry[]> {
  return await invoke<SkillEntry[]>("list_skills", { project_id: projectId });
}

/** The `/`-token an author should insert for an entry (no leading slash): the
 *  qualified `namespace:name` when flagged + namespaced, else the bare name. */
export function insertForm(entry: SkillEntry): string {
  if (entry.qualified && entry.namespace) return `${entry.namespace}:${entry.name}`;
  return entry.name;
}
