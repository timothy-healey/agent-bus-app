import { invoke } from "@tauri-apps/api/core";

export type RunnerKind = "claude-cli" | "anthropic-api";

export type EffortMode =
  | { mode: "off" }
  | { mode: "standard" }
  | { mode: "extended-low" }
  | { mode: "extended-high" }
  | { mode: "custom"; budget_tokens: number };

export interface RunnerConfig {
  kind: RunnerKind;
  model: string;
  effort: EffortMode;
  api_key_env?: string | null;
}

export interface Scope {
  reads: string[];
  writes: string[];
  tools: string[];
}

export interface Routes {
  on_approve?: string | null;
  on_revise?: string | null;
  on_reject?: string | null;
}

export interface Workers {
  default: number;
  max: number;
}

export interface Team {
  id: string;
  name: string;
  prompt: string;
  runner: RunnerConfig;
  scope: Scope;
  outputs: Routes;
  workers: Workers;
}

export interface Gate {
  id: string;
  label: string;
  downstream: string;
}

export interface Escalation {
  id: string;
  triggers: string[];
}

export interface Fork {
  id: string;
  lanes: string[];
}

export interface Join {
  id: string;
  waits_for: string[];
  downstream: string;
}

export interface Pipeline {
  id: string;
  name: string;
  description: string;
  schema_version: number;
  teams: Team[];
  gates: Gate[];
  escalations: Escalation[];
  forks: Fork[];
  joins: Join[];
}

export interface TemplateInfo {
  id: string;
  name: string;
}

export async function listTemplates(): Promise<TemplateInfo[]> {
  return await invoke<TemplateInfo[]>("pipeline_list_templates");
}

export async function listPipelines(projectRoot: string): Promise<string[]> {
  return await invoke<string[]>("pipeline_list", { project_root: projectRoot });
}

export async function loadPipeline(projectRoot: string, id: string): Promise<Pipeline> {
  return await invoke<Pipeline>("pipeline_load", { project_root: projectRoot, id });
}

export async function instantiateTemplate(
  projectRoot: string,
  templateId: string,
): Promise<Pipeline> {
  return await invoke<Pipeline>("pipeline_instantiate_template", {
    project_root: projectRoot,
    template_id: templateId,
  });
}
