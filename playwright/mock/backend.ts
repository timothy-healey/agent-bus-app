/// S5 mock backend — a tiny, hand-written, in-memory fake of the OHS the React
/// app calls over Tauri IPC. The E2E Vite build (vite.config.e2e.ts) aliases
/// `@tauri-apps/api/{core,event}` to `tauri-core.ts` / `tauri-event.ts`, which
/// delegate here. The real `src/ipc/*.ts` wrappers run unchanged on top.
///
/// CONTRACT NOTE (the hand-written-mock risk, see README): every response shape is
/// typed against the app's own `src/ipc/*.ts` types, so `tsc` in this dir catches
/// drift. The mock is a fake, NOT the real Rust backend — a green suite proves the
/// UI flow against THESE shapes, not against the live OHS.
import type { Project } from "../../src/ipc/workspace";
import type {
  DraftPipeline,
  Pipeline,
  SeedTemplateSummary,
} from "../../src/ipc/pipeline";
import type {
  Task,
  Run,
  StoreOccupancy,
  InvocationRow,
  BrakeState,
} from "../../src/ipc/runtime";
import type { UsageSnapshot } from "../../src/ipc/usage";
import type { SkillEntry } from "../../src/ipc/skills";
import type { Conversation } from "../../src/ipc/terminal";

// ---------------------------------------------------------------------------
// Event bus (mirrors @tauri-apps/api/event listen/emit)
// ---------------------------------------------------------------------------

type Listener = (payload: unknown) => void;
const listeners = new Map<string, Set<Listener>>();

export function busListen(event: string, cb: Listener): () => void {
  let set = listeners.get(event);
  if (!set) {
    set = new Set();
    listeners.set(event, set);
  }
  set.add(cb);
  return () => {
    set?.delete(cb);
  };
}

export function busEmit(event: string, payload: unknown): void {
  const set = listeners.get(event);
  if (!set) return;
  for (const cb of [...set]) cb(payload);
}

// ---------------------------------------------------------------------------
// Mutable in-memory state
// ---------------------------------------------------------------------------

export interface MockState {
  projects: Project[];
  /// Resolved pipeline per project root (what pipeline_load returns). The board +
  /// canvas read this. Keyed by project root_path.
  pipelinesByRoot: Record<string, Pipeline>;
  /// Pipeline ids per project root (pipeline_list).
  pipelineIdsByRoot: Record<string, string[]>;
  seedTemplates: SeedTemplateSummary[];
  /// The DraftPipeline a seed template expands to (seed_template_cmd).
  seedDrafts: Record<string, DraftPipeline>;
  tasks: Task[];
  runs: Run[];
  /// Per-run store occupancy (run_store_occupancy), keyed by run id.
  occupancyByRun: Record<string, StoreOccupancy[]>;
  /// Per-task invocation audit trail (list_invocations), keyed by task id.
  invocationsByTask: Record<string, InvocationRow[]>;
  skills: SkillEntry[];
  usage: UsageSnapshot;
  brake: BrakeState;
  conversation: Conversation | null;
  /// best_effort_validate result (issues list). Empty = valid.
  validationIssues: string[];
}

let SEQ = 1;
const nextId = (p: string) => `${p}-${SEQ++}`;
const NOW = () => Math.floor(Date.now() / 1000);

function emptyUsage(): UsageSnapshot {
  return {
    window_total: 0,
    window_budget: 1_000_000,
    window_pct: 0,
    band: "safe",
    burn_per_min: 0,
    window_secs: 18_000,
    reset_in_secs: null,
    est_brake_at: null,
    by_team: [],
    tokens_by_task: {},
    braked: false,
    auto_meter_enabled: true,
  };
}

function makeProject(over: Partial<Project> = {}): Project {
  const id = over.id ?? nextId("proj");
  return {
    id,
    name: over.name ?? "Mock project",
    root_path: over.root_path ?? `/mock/${id}`,
    target_repo: over.target_repo ?? null,
    skill_sources: over.skill_sources ?? [],
    active_pipeline_id: over.active_pipeline_id ?? null,
    created_at: over.created_at ?? NOW(),
    updated_at: over.updated_at ?? NOW(),
  };
}

/// The pipeline a fresh project lands on (also the board's lane source). Two teams
/// (writer → reviewer) + a human gate + a terminal escalation, so the board shows
/// team lanes with store/pool indicators and the recovery classifier has a
/// terminal escalation to read.
export function demoPipeline(id = "demo"): Pipeline {
  return {
    id,
    name: "Demo pipeline",
    description: "A writer hands off to a reviewer behind a human gate.",
    schema_version: 3,
    teams: [
      {
        id: "writer",
        name: "Writer",
        prompt: "Draft the deliverable.",
        runner: { kind: "claude-cli", model: "claude-opus-4-8", effort: { mode: "standard" } },
        scope: { reads: [], writes: ["artifacts/"], tools: [] },
        outputs: { on_approve: "reviewer" },
        workers: { min: 1, max: 3 },
        role: "producer",
        store: { capacity: 8 },
      },
      {
        id: "reviewer",
        name: "Reviewer",
        prompt: "Review the draft and gate it.",
        runner: { kind: "claude-cli", model: "claude-opus-4-8", effort: { mode: "standard" } },
        scope: { reads: ["artifacts/"], writes: [], tools: [] },
        outputs: { on_approve: "human-gate", on_revise: "writer", on_reject: "escalate" },
        workers: { min: 1, max: 2 },
        role: "reviewer",
        store: { capacity: 4 },
      },
    ],
    gates: [{ id: "human-gate", label: "Human gate", downstream: "done" }],
    escalations: [{ id: "escalate", triggers: ["reject", "error"] }],
    forks: [],
    joins: [],
  };
}

/// The DraftPipeline a seed template expands to (mirrors what the wizard renders on
/// the canvas → team nodes + store nodes + a forward edge).
export function demoDraft(): DraftPipeline {
  return {
    id: "demo",
    name: "",
    description: "A writer hands off to a reviewer behind a human gate.",
    schema_version: 3,
    teams: [
      {
        id: "writer",
        name: "Writer",
        prompt_body: "Draft the deliverable.",
        runner: { kind: "claude-cli", model: "claude-opus-4-8", effort: { mode: "standard" }, api_key_env: null },
        scope: { reads: [], writes: ["artifacts/"], tools: [] },
        outputs: { on_approve: "reviewer" },
        workers: { min: 1, max: 3 },
        role: "producer",
        store: { capacity: 8 },
      },
      {
        id: "reviewer",
        name: "Reviewer",
        prompt_body: "Review the draft and gate it.",
        runner: { kind: "claude-cli", model: "claude-opus-4-8", effort: { mode: "standard" }, api_key_env: null },
        scope: { reads: ["artifacts/"], writes: [], tools: [] },
        outputs: { on_approve: "human-gate", on_revise: "writer", on_reject: "escalate" },
        workers: { min: 1, max: 2 },
        role: "reviewer",
        store: { capacity: 4 },
      },
    ],
    forks: [],
    joins: [],
    gates: [{ id: "human-gate", label: "Human gate", downstream: "done" }],
    escalations: [{ id: "escalate", triggers: ["reject", "error"] }],
  };
}

export function initialState(): MockState {
  return {
    projects: [],
    pipelinesByRoot: {},
    pipelineIdsByRoot: {},
    seedTemplates: [
      { id: "writer-reviewer", name: "Writer + Reviewer", description: "A writer hands off to a reviewer behind a human gate." },
      { id: "solo", name: "Solo writer", description: "A single producing team." },
    ],
    seedDrafts: { "writer-reviewer": demoDraft(), solo: demoDraft() },
    tasks: [],
    runs: [],
    occupancyByRun: {},
    invocationsByTask: {},
    skills: [
      { name: "ddd-council", kind: "skill", namespace: null, description: "DDD council", verbs: [], source: "global", qualified: false },
    ],
    usage: emptyUsage(),
    brake: { on: false, reason: null },
    conversation: null,
    validationIssues: [],
  };
}

let state: MockState = initialState();

export function getState(): MockState {
  return state;
}

/// Per-test seed: deep-merge a partial state over a fresh baseline. The spec calls
/// this through `window.__E2E__.seed(...)` before the app mounts (addInitScript).
export function seed(partial: Partial<MockState>): void {
  state = { ...initialState(), ...partial };
}

// ---------------------------------------------------------------------------
// Command handler map (mirrors @tauri-apps/api/core invoke)
// ---------------------------------------------------------------------------

type Args = Record<string, unknown> | undefined;
type Handler = (args: Args) => unknown;

/// Per-command overrides installed by a spec via `window.__E2E__.on(cmd, fn)`.
const overrides = new Map<string, Handler>();

export function on(cmd: string, fn: Handler): void {
  overrides.set(cmd, fn);
}

function project(id: string): Project | undefined {
  return state.projects.find((p) => p.id === id);
}

const handlers: Record<string, Handler> = {
  // --- Workspace --------------------------------------------------------------
  workspace_list_projects: () => state.projects,
  workspace_get_project: (a) => project(String(a?.id)) ?? null,
  workspace_set_active_pipeline: () => null,
  workspace_remove_project: (a) => {
    state.projects = state.projects.filter((p) => p.id !== a?.id);
    return null;
  },
  workspace_set_target_repo: () => null,
  workspace_set_skill_sources: () => null,
  activate_project: () => null,
  read_artifact: () => "# Mock artifact\n\nSeeded body for the E2E run.",
  git_config_get: () => ({ author_name: "Mock Author", author_email: "mock@example.com" }),
  git_config_set: (a) => ({ author_name: String(a?.author_name ?? ""), author_email: String(a?.author_email ?? "") }),
  list_worktrees: () => [],
  remove_worktree: () => null,
  list_dir: () => [],

  // --- Pipeline authoring -----------------------------------------------------
  list_seed_templates_cmd: () => state.seedTemplates,
  seed_template_cmd: (a) => state.seedDrafts[String(a?.id)] ?? demoDraft(),
  best_effort_validate_cmd: () => state.validationIssues,
  pipeline_list: (a) => state.pipelineIdsByRoot[String(a?.project_root)] ?? [],
  pipeline_load: (a) => {
    const root = String(a?.project_root);
    return state.pipelinesByRoot[root] ?? demoPipeline();
  },
  pipeline_to_draft_cmd: () => demoDraft(),
  save_pipeline_edits: () => null,
  kickoff_generate_cmd: () => demoDraft(),
  design_session_turn_cmd: (a) => ({
    reply_text: "ok",
    updated_draft: (a?.draft as DraftPipeline) ?? demoDraft(),
    issues: [],
  }),
  create_project_from_draft: (a) => {
    const draft = a?.draft as DraftPipeline | undefined;
    const proj = makeProject({
      name: String(a?.name ?? "New project"),
      root_path: String(a?.root ?? `/mock/${nextId("root")}`),
      target_repo: (a?.target_repo as string | null) ?? null,
      active_pipeline_id: draft?.id ?? "demo",
    });
    // create MUTATES: add the project + register a resolved pipeline at its root so
    // the board has lanes the moment the wizard lands on it.
    state.projects = [proj, ...state.projects];
    const pipeline = draft ? draftToResolvedPipeline(draft) : demoPipeline();
    state.pipelinesByRoot[proj.root_path] = pipeline;
    state.pipelineIdsByRoot[proj.root_path] = [pipeline.id];
    busEmit("run-changed", "");
    return proj;
  },

  // --- Runtime: runs + tasks --------------------------------------------------
  list_runs: (a) => state.runs.filter((r) => r.project_id === String(a?.project_id)),
  run_store_occupancy: (a) => state.occupancyByRun[String(a?.run_id)] ?? [],
  list_tasks: () => state.tasks,
  start_run: (a) => {
    const proj = state.projects[0];
    const run: Run = {
      id: nextId("run"),
      pipeline: "demo",
      project_id: proj?.id ?? "proj-0",
      generator_dry: false,
      completed: false,
      created_at: NOW(),
    };
    state.runs = [run, ...state.runs];
    busEmit("run-changed", run.id);
    busEmit("task-changed", "");
    void a;
    return run;
  },
  inject_topic: (a) => makeTask({ topic: String(a?.topic ?? "injected") }),

  // --- Gate verdicts ----------------------------------------------------------
  approve_gate: (a) => settleTask(String(a?.task_id), "done"),
  revise_gate: (a) => settleTask(String(a?.task_id), "revising"),
  reject_gate: (a) => settleTask(String(a?.task_id), "needs_human"),

  // --- L2/L3 recovery ---------------------------------------------------------
  list_invocations: (a) => state.invocationsByTask[String(a?.task_id)] ?? [],
  retry_task: (a) => {
    const id = String(a?.task_id);
    const t = state.tasks.find((x) => x.id === id);
    if (t) {
      // L2 retry: requeue at the stage that escalated it; reset attempts.
      t.state = "queued";
      t.attempts = 0;
      t.updated_at = NOW();
      busEmit("task-changed", id);
    }
    return t ?? null;
  },
  force_advance: (a) => settleTask(String(a?.task_id), "queued", (t) => { t.attempts = 0; }),
  abandon_task: (a) => settleTask(String(a?.task_id), "done"),
  accept_task: (a) => settleTask(String(a?.task_id), "done"),

  // --- Review -----------------------------------------------------------------
  record_verdict: (a) => ({ task_id: String(a?.task_id), verdict: a?.verdict, comment_count: 0 }),
  add_comment: (a) => makeComment(a),
  list_comments: () => [],
  delete_comment: () => null,
  reanchor_comments: () => [],

  // --- Skills / usage / brake / secrets / terminal / models -------------------
  list_skills: () => state.skills,
  usage_snapshot: () => state.usage,
  usage_set_budget: (a) => { state.usage = { ...state.usage, window_budget: Number(a?.budget ?? 0) }; return state.usage; },
  usage_set_auto_meter: (a) => { state.usage = { ...state.usage, auto_meter_enabled: !!a?.enabled }; return state.usage; },
  brake_state: () => state.brake,
  brake_on: (a) => { state.brake = { on: true, reason: (a?.reason as string | null) ?? "manual" }; busEmit("usage-changed", null); return state.brake; },
  brake_off: () => { state.brake = { on: false, reason: null }; busEmit("usage-changed", null); return state.brake; },
  scale_team: () => 1,
  runner_set_api_key: () => null,
  runner_get_api_key_status: () => false,
  runner_clear_api_key: () => null,
  test_model: () => ({ status: "ok", message: "probe ok" }),
  send_message: (a) => {
    const text = String(a?.input ?? "");
    const conv: Conversation = state.conversation ?? {
      project_id: state.projects[0]?.id ?? "proj-0",
      session_id: "mock-session",
      started_at: NOW(),
      last_message_at: NOW(),
      turns: [],
      summary_of_prior_sessions: null,
      history_budget_tokens: 100_000,
    };
    conv.turns = [
      ...conv.turns,
      { role: "user", text, tool_calls: [], at: NOW() },
      { role: "assistant", text: `echo: ${text}`, tool_calls: [], at: NOW() },
    ];
    conv.last_message_at = NOW();
    state.conversation = conv;
    return conv;
  },
  get_conversation: () => state.conversation,
};

// --- helpers used by handlers ------------------------------------------------

function makeTask(over: Partial<Task> = {}): Task {
  const id = over.id ?? nextId("task");
  const t: Task = {
    id,
    project_id: over.project_id ?? state.projects[0]?.id ?? "proj-0",
    pipeline: over.pipeline ?? "demo",
    topic: over.topic ?? "Mock work-item",
    target_repo: over.target_repo ?? null,
    target_scope: over.target_scope ?? null,
    current_stage: over.current_stage ?? "writer",
    state: over.state ?? "queued",
    attempts: over.attempts ?? 0,
    parent_artifact: over.parent_artifact ?? null,
    review_artifact: over.review_artifact ?? null,
    created_at: over.created_at ?? NOW(),
    updated_at: over.updated_at ?? NOW(),
    run_id: over.run_id ?? state.runs[0]?.id ?? null,
    item_key: over.item_key ?? null,
  };
  state.tasks = [...state.tasks, t];
  busEmit("task-changed", id);
  return t;
}

function settleTask(id: string, next: Task["state"], mutate?: (t: Task) => void): Task | null {
  const t = state.tasks.find((x) => x.id === id);
  if (!t) return null;
  t.state = next;
  t.updated_at = NOW();
  mutate?.(t);
  busEmit("task-changed", id);
  return t;
}

function makeComment(a: Args) {
  return {
    id: nextId("comment"),
    task_id: String(a?.task_id ?? ""),
    artifact_path: String(a?.artifact_path ?? ""),
    anchor_text: (a?.anchor_text as string | null) ?? null,
    anchor_offset: (a?.anchor_offset as number | null) ?? null,
    note: String(a?.note ?? ""),
    kind: a?.kind,
    created_at: NOW(),
  };
}

/// Resolve a DraftPipeline into the board's Pipeline shape (drops draft-only
/// fields, supplies the `prompt` + resolved `runner`). Mirrors the backend's
/// to_pipeline well enough for the board lanes.
function draftToResolvedPipeline(d: DraftPipeline): Pipeline {
  return {
    id: d.id || "demo",
    name: d.name || "Pipeline",
    description: d.description,
    schema_version: d.schema_version,
    teams: d.teams.map((t) => ({
      id: t.id,
      name: t.name,
      prompt: t.prompt_body,
      runner: t.runner,
      scope: t.scope,
      outputs: t.outputs,
      workers: t.workers,
      role: t.role,
      store: t.store,
    })),
    gates: d.gates,
    escalations: d.escalations,
    forks: d.forks,
    joins: d.joins,
  };
}

// ---------------------------------------------------------------------------
// Public invoke entry point (used by tauri-core.ts)
// ---------------------------------------------------------------------------

export async function invoke<T = unknown>(cmd: string, args?: Args): Promise<T> {
  const override = overrides.get(cmd);
  if (override) return Promise.resolve(override(args) as T);
  const handler = handlers[cmd];
  if (handler) return Promise.resolve(handler(args) as T);
  // Unknown command → benign default + a console warning so a newly-added command
  // surfaces during the run instead of crashing the UI.
  // eslint-disable-next-line no-console
  console.warn(`[e2e-mock] unhandled command: ${cmd}`, args);
  return Promise.resolve(null as T);
}

// ---------------------------------------------------------------------------
// window.__E2E__ install — read a global the spec sets via addInitScript.
// ---------------------------------------------------------------------------

declare global {
  interface Window {
    __E2E__?: {
      seed: (state: Partial<MockState>) => void;
      on: (cmd: string, fn: Handler) => void;
      emit: (event: string, payload: unknown) => void;
      getState: () => MockState;
    };
    /// A spec may stash a pre-mount seed payload here (addInitScript runs before
    /// the app bundle); the mock applies it on module load.
    __E2E_SEED__?: Partial<MockState>;
  }
}

if (typeof window !== "undefined") {
  if (window.__E2E_SEED__) {
    try {
      seed(window.__E2E_SEED__);
    } catch (e) {
      // eslint-disable-next-line no-console
      console.warn("[e2e-mock] seed from __E2E_SEED__ failed", e);
    }
  }
  window.__E2E__ = { seed, on, emit: busEmit, getState };
}
