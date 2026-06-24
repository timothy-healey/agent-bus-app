import type { DraftPipeline, DraftTeam, EffortMode, Fork, Gate, Join, Workers } from "../ipc/pipeline";

/** Default WIP capacity for a new team's input store (mirrors the backend
 *  default authored in WiringStep.draftToPipeline before this chunk). */
const DEFAULT_STORE_CAPACITY = 8;

export const WIZARD_STEPS = ["basics", "teams", "prompts", "wiring", "review"] as const;
export type WizardStep = (typeof WIZARD_STEPS)[number];

const SCHEMA_VERSION = 3;

export function emptyDraft(): DraftPipeline {
  return {
    id: "",
    name: "",
    description: "",
    schema_version: SCHEMA_VERSION,
    teams: [],
    forks: [],
    joins: [],
    gates: [],
    escalations: [],
  };
}

function defaultTeam(id: string, name: string): DraftTeam {
  return {
    id,
    name,
    prompt_body: "",
    runner: { kind: "claude-cli", model: "claude-opus-4-8", effort: { mode: "standard" }, api_key_env: null },
    scope: { reads: [], writes: [], tools: [] },
    outputs: {},
    workers: { min: 1, max: 1 },
    role: "producer",
    store: { capacity: DEFAULT_STORE_CAPACITY },
  };
}

function mapTeams(d: DraftPipeline, f: (t: DraftTeam) => DraftTeam): DraftPipeline {
  return { ...d, teams: d.teams.map(f) };
}

export function addTeam(d: DraftPipeline, id: string, name: string): DraftPipeline {
  if (d.teams.some((t) => t.id === id)) return d;
  return { ...d, teams: [...d.teams, defaultTeam(id, name)] };
}

export function removeTeam(d: DraftPipeline, id: string): DraftPipeline {
  return { ...d, teams: d.teams.filter((t) => t.id !== id) };
}

export function renameTeam(d: DraftPipeline, id: string, name: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, name } : t));
}

export function setPromptBody(d: DraftPipeline, id: string, body: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, prompt_body: body } : t));
}

export function setTeamModel(d: DraftPipeline, id: string, model: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, runner: { ...t.runner, model } } : t));
}

/// Parse a comma-separated input into a trimmed, non-empty string list (the
/// shape Scope.tools/reads/writes use).
function parseCsv(raw: string): string[] {
  return raw.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
}

export function setTeamEffort(d: DraftPipeline, id: string, effort: EffortMode): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, runner: { ...t.runner, effort } } : t));
}

/// Set a team's authoring role (vet F8). Producer = one hand-off edge; reviewer =
/// the approve·revise·reject verdict triple.
export function setTeamRole(d: DraftPipeline, id: string, role: "producer" | "reviewer"): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, role } : t));
}

/// Set a team's input-store WIP capacity (chunk ①). Floored at 1; non-finite → 1.
export function setTeamStoreCapacity(d: DraftPipeline, id: string, capacity: number): DraftPipeline {
  const cap = Number.isFinite(capacity) ? Math.max(1, Math.floor(capacity)) : 1;
  return mapTeams(d, (t) => (t.id === id ? { ...t, store: { capacity: cap } } : t));
}

/// Set a team's Scale (min · max). Clamps min ≥ 1 and max ≥ min so the dial can
/// never author an invalid range.
export function setTeamWorkers(d: DraftPipeline, id: string, workers: Workers): DraftPipeline {
  const min = Number.isFinite(workers.min) ? Math.max(1, Math.floor(workers.min)) : 1;
  const max = Number.isFinite(workers.max) ? Math.max(min, Math.floor(workers.max)) : min;
  return mapTeams(d, (t) => (t.id === id ? { ...t, workers: { min, max } } : t));
}

export function setTeamTools(d: DraftPipeline, id: string, raw: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, scope: { ...t.scope, tools: parseCsv(raw) } } : t));
}

export function setTeamReads(d: DraftPipeline, id: string, raw: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, scope: { ...t.scope, reads: parseCsv(raw) } } : t));
}

export function setTeamWrites(d: DraftPipeline, id: string, raw: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, scope: { ...t.scope, writes: parseCsv(raw) } } : t));
}

export function addGate(d: DraftPipeline, id: string, label: string, downstream: string): DraftPipeline {
  if (d.gates.some((g) => g.id === id)) return d;
  const gate: Gate = { id, label, downstream };
  return { ...d, gates: [...d.gates, gate] };
}

export function removeGate(d: DraftPipeline, id: string): DraftPipeline {
  return { ...d, gates: d.gates.filter((g) => g.id !== id) };
}

export function setTeamApprove(d: DraftPipeline, teamId: string, target: string | null): DraftPipeline {
  return mapTeams(d, (t) => (t.id === teamId ? { ...t, outputs: { ...t.outputs, on_approve: target } } : t));
}

/// Create a paired fork + join in one action (W4). A fork must have >= 2 lanes
/// (lane teams); the join waits on those same lanes and routes to `downstream`.
/// No-op on <2 lanes or a duplicate fork/join id, so the user can never author
/// an unpaired or malformed fork.
export function addForkJoin(
  d: DraftPipeline,
  forkId: string,
  joinId: string,
  lanes: string[],
  downstream: string,
): DraftPipeline {
  if (lanes.length < 2) return d;
  if (d.forks.some((f) => f.id === forkId)) return d;
  if (d.joins.some((j) => j.id === joinId)) return d;
  const fork: Fork = { id: forkId, lanes };
  const join: Join = { id: joinId, waits_for: lanes, downstream };
  return { ...d, forks: [...d.forks, fork], joins: [...d.joins, join] };
}

export function removeForkJoin(d: DraftPipeline, forkId: string, joinId: string): DraftPipeline {
  return { ...d, forks: d.forks.filter((f) => f.id !== forkId), joins: d.joins.filter((j) => j.id !== joinId) };
}
