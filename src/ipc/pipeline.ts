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

export async function listPipelines(projectRoot: string): Promise<string[]> {
  return await invoke<string[]>("pipeline_list", { project_root: projectRoot });
}

export async function loadPipeline(projectRoot: string, id: string): Promise<Pipeline> {
  return await invoke<Pipeline>("pipeline_load", { project_root: projectRoot, id });
}

export interface DraftTeam {
  id: string;
  name: string;
  prompt_body: string;
  runner: RunnerConfig;
  scope: Scope;
  outputs: Routes;
  workers: Workers;
}

export interface DraftPipeline {
  id: string;
  name: string;
  description: string;
  schema_version: number;
  teams: DraftTeam[];
  forks: Fork[];
  joins: Join[];
  escalations: Escalation[];
}

export type Step = "teams" | "prompts" | "wiring";

export interface TurnResult {
  reply_text: string;
  updated_draft: DraftPipeline;
}

export async function kickoffGenerate(sessionId: string, description: string): Promise<DraftPipeline> {
  return await invoke<DraftPipeline>("kickoff_generate_cmd", {
    session_id: sessionId,
    description,
  });
}

export async function designSessionTurn(
  sessionId: string,
  step: Step,
  draft: DraftPipeline,
  userMessage: string,
): Promise<TurnResult> {
  return await invoke<TurnResult>("design_session_turn_cmd", {
    session_id: sessionId,
    step,
    draft,
    user_message: userMessage,
  });
}

export async function createProjectFromDraft(
  name: string,
  root: string,
  draft: DraftPipeline,
): Promise<{ id: string; name: string; root_path: string; active_pipeline_id: string | null; created_at: number; updated_at: number }> {
  return await invoke("create_project_from_draft", { name, root, draft });
}
