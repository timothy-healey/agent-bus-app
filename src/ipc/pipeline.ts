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

/** A team's runner as authored (R5): every field optional so a team can override
 *  just the model and inherit the rest from Pipeline.defaults. */
export interface TeamRunnerConfig {
  kind?: RunnerKind | null;
  model?: string | null;
  effort?: EffortMode | null;
  api_key_env?: string | null;
}

/** Pipeline-level runner defaults teams inherit when they omit their own (R5). */
export interface PipelineDefaults {
  default_runner?: RunnerKind | null;
  default_model?: string | null;
  default_effort?: EffortMode | null;
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
  min: number;
  max: number;
}

/** A team's bounded input store (runtime redesign). `capacity` is the WIP limit
 *  that drives backpressure to the upstream. */
export interface Store {
  capacity: number;
}

export interface Team {
  id: string;
  name: string;
  prompt: string;
  // Authored form is a partial override (R5); after pipeline_load (which resolves
  // pipeline defaults backend-side) this is the full RunnerConfig. Optional on the
  // wire because an authored team may omit it to inherit the pipeline default.
  runner?: TeamRunnerConfig | null;
  scope: Scope;
  outputs: Routes;
  workers: Workers;
  role: "producer" | "reviewer";
  store: Store;
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
  /** Early-cancel policy (P2). When true, the join resolves to needs-human the
   *  moment one lane fails, cancelling the outstanding lanes. Optional; absent =
   *  the full-barrier default. Backend field is `cancel_on_reject` (serde default
   *  false). */
  cancel_on_reject?: boolean;
  /** Quorum (P3): proceed to downstream once `quorum` lanes approve (N-of-M);
   *  resolve to needs-human once reaching it is impossible. Omitted/undefined =
   *  all-must-approve (the default barrier). Backend field is `quorum`
   *  (`Option<u32>`, serde-skipped when None). When set, it governs success and
   *  `cancel_on_reject` is ignored. */
  quorum?: number;
}

export interface Pipeline {
  id: string;
  name: string;
  description: string;
  schema_version: number;
  defaults?: PipelineDefaults | null;
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
  /** Role (vet F8). Producer emits a single hand-off edge; reviewer emits the
   *  approve·revise·reject verdict triple. The backend `DraftTeam` now carries
   *  this and always serializes it (serde default Producer), so it is required
   *  on the wire and round-trips through to_pipeline/from_pipeline. */
  role: "producer" | "reviewer";
  /** Bounded-input store (chunk ①). `capacity` is the WIP limit. The backend
   *  `DraftTeam` now carries this and always serializes it (serde default
   *  capacity 8), so it is required on the wire and round-trips. */
  store: Store;
}

export interface DraftPipeline {
  id: string;
  name: string;
  description: string;
  schema_version: number;
  teams: DraftTeam[];
  forks: Fork[];
  joins: Join[];
  gates: Gate[];
  escalations: Escalation[];
}

export type Step = "teams" | "prompts" | "wiring";

export interface TurnResult {
  reply_text: string;
  updated_draft: DraftPipeline;
  issues: string[];
}

export async function kickoffGenerate(sessionId: string, description: string): Promise<DraftPipeline> {
  return await invoke<DraftPipeline>("kickoff_generate_cmd", {
    session_id: sessionId,
    description,
  });
}

/** A bundled seed template summary for the kickoff picker (A2). */
export interface SeedTemplateSummary {
  id: string;
  name: string;
  description: string;
}

export async function listSeedTemplates(): Promise<SeedTemplateSummary[]> {
  return await invoke<SeedTemplateSummary[]>("list_seed_templates_cmd");
}

/** Return a populated DraftPipeline seed for a template id; the wizard refines it. */
export async function seedTemplate(id: string): Promise<DraftPipeline> {
  return await invoke<DraftPipeline>("seed_template_cmd", { id });
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

/** G5 — regenerate ONE team's responsibility prompt by running the existing
 *  Step::Prompts design-session logic over the `llm_chat` seam (NOT a forked
 *  session). Returns the regenerated prompt body for `teamId`, or null when the
 *  turn produced no usable slice (the model's prose is surfaced separately). The
 *  bounded-repair discipline lives in the backend turn. */
export async function regenerateTeamPrompt(
  sessionId: string,
  draft: DraftPipeline,
  teamId: string,
): Promise<string | null> {
  const team = draft.teams.find((t) => t.id === teamId);
  const name = team?.name || teamId;
  // The Step::Prompts system prompt expects a single team_id; name it explicitly.
  const message =
    `Write the responsibility prompt for the team with id "${teamId}" (${name}). ` +
    `Use the current draft for context. Emit a PromptSlice for that team only.`;
  const result = await designSessionTurn(sessionId, "prompts", draft, message);
  const updated = result.updated_draft.teams.find((t) => t.id === teamId);
  // Only treat it as regenerated when the body actually changed to something
  // non-empty (a no-op turn leaves the draft unchanged).
  const next = updated?.prompt_body ?? "";
  if (next.trim() === "" || next === (team?.prompt_body ?? "")) return null;
  return next;
}

export async function bestEffortValidate(draft: DraftPipeline): Promise<string[]> {
  return await invoke<string[]>("best_effort_validate_cmd", { draft });
}

export async function createProjectFromDraft(
  name: string,
  root: string,
  draft: DraftPipeline,
  targetRepo?: string | null,
): Promise<{ id: string; name: string; root_path: string; target_repo: string | null; active_pipeline_id: string | null; created_at: number; updated_at: number }> {
  return await invoke("create_project_from_draft", { name, root, draft, target_repo: targetRepo ?? null });
}

/** Load a project's pipeline (resolved) into an editable DraftPipeline, reading
 *  each team's prompt body back from disk (A1 editor seed). */
export async function pipelineToDraft(
  projectId: string,
  projectRoot: string,
  id: string,
): Promise<DraftPipeline> {
  return await invoke<DraftPipeline>("pipeline_to_draft_cmd", {
    project_id: projectId,
    project_root: projectRoot,
    id,
  });
}

/** Hard-validate then overwrite the project's active pipeline YAML + prompt files
 *  (A1). Throws (no write) on a hard-validation failure. */
export async function savePipelineEdits(projectId: string, draft: DraftPipeline): Promise<void> {
  await invoke<void>("save_pipeline_edits", { project_id: projectId, draft });
}
