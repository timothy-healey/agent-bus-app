import type { DraftPipeline, DraftTeam, Effort, Fork, Gate, Join, Scope, SimpleGrant, Workers } from "../ipc/pipeline";
import { DEFAULT_TEAM_MODEL, findModel, type ModelList } from "../ipc/models";

/** Default WIP capacity for a new team's input store (mirrors the backend
 *  default). */
const DEFAULT_STORE_CAPACITY = 8;

/// The new-project wizard steps. The Teams/Prompts/Wiring form trio collapsed
/// into one interactive Canvas step (graph-builder chunk ②); Basics (incl.
/// Generate) and Review/Create remain.
export const WIZARD_STEPS = ["basics", "canvas", "review"] as const;
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
    runner: { kind: "claude-cli", model: DEFAULT_TEAM_MODEL, api_key_env: null },
    scope: { reads: [], writes: [], grants: [], plugins: [] },
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

/// Set a team's model. If the list says the new model doesn't support the
/// team's effort level, effort snaps to Default; `snapped` is the dropped level
/// (null when nothing changed) so the caller can say so. With no list, or a
/// model the list lacks, effort is left alone.
export function setTeamModel(
  d: DraftPipeline,
  id: string,
  model: string,
  list: ModelList | null,
): { draft: DraftPipeline; snapped: string | null } {
  const team = d.teams.find((t) => t.id === id);
  const level = team?.runner.effort;
  const option = findModel(list, model);
  const snap = level !== undefined && option !== undefined && !option.effort_levels.includes(level);
  const draft = mapTeams(d, (t) => {
    if (t.id !== id) return t;
    const runner = { ...t.runner, model };
    if (snap) delete runner.effort;
    return { ...t, runner };
  });
  return { draft, snapped: snap ? (level ?? null) : null };
}

/// Parse a comma-separated input into a trimmed, non-empty string list (the
/// shape Scope.reads/writes use).
function parseCsv(raw: string): string[] {
  return raw.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
}

/// Set a team's effort level; `undefined` is Default and removes the key, the
/// shape the backend writes for Default.
export function setTeamEffort(d: DraftPipeline, id: string, effort: Effort | undefined): DraftPipeline {
  return mapTeams(d, (t) => {
    if (t.id !== id) return t;
    const runner = { ...t.runner, effort };
    if (effort === undefined) delete runner.effort;
    return { ...t, runner };
  });
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

/// Turn one simple grant on or off for a team (never duplicated).
export function setTeamGrant(d: DraftPipeline, id: string, grant: SimpleGrant, on: boolean): DraftPipeline {
  return mapTeams(d, (t) => {
    if (t.id !== id) return t;
    const rest = t.scope.grants.filter((g) => g !== grant);
    return { ...t, scope: { ...t.scope, grants: on ? [...rest, grant] : rest } };
  });
}

const PATTERN = /^bash\((.*)\)$/i;

/// The Bash patterns a scope grants, without their `bash(...)` wrapper.
export function bashPatterns(scope: Scope): string[] {
  return scope.grants.map((g) => PATTERN.exec(g)?.[1]).filter((p): p is string => p !== undefined);
}

/// Replace a team's Bash pattern grants from a comma-separated list. Each
/// entry may be written bare (`git diff:*`) or wrapped (`Bash(git diff:*)`).
export function setTeamBashPatterns(d: DraftPipeline, id: string, raw: string): DraftPipeline {
  const patterns = parseCsv(raw).map((p) => PATTERN.exec(p)?.[1]?.trim() ?? p).filter((p) => p.length > 0);
  return mapTeams(d, (t) => {
    if (t.id !== id) return t;
    const simple = t.scope.grants.filter((g) => !PATTERN.test(g));
    return { ...t, scope: { ...t.scope, grants: [...simple, ...patterns.map((p) => `bash(${p})`)] } };
  });
}

/// Turn one plugin on or off for a team (never duplicated).
export function setTeamPlugin(d: DraftPipeline, id: string, name: string, on: boolean): DraftPipeline {
  return mapTeams(d, (t) => {
    if (t.id !== id) return t;
    const rest = t.scope.plugins.filter((p) => p !== name);
    return { ...t, scope: { ...t.scope, plugins: on ? [...rest, name] : rest } };
  });
}

/// Add a plugin to a team if it is not there yet (a skill picked in the prompt
/// brings its plugin along).
export function addTeamPlugin(d: DraftPipeline, id: string, name: string): DraftPipeline {
  const team = d.teams.find((t) => t.id === id);
  if (!team || team.scope.plugins.includes(name)) return d;
  return setTeamPlugin(d, id, name, true);
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

function mapJoin(d: DraftPipeline, joinId: string, f: (j: Join) => Join): DraftPipeline {
  return { ...d, joins: d.joins.map((j) => (j.id === joinId ? f(j) : j)) };
}

/// Set/clear a join's N-of-M quorum (AU1/P3). A number sets the quorum; `undefined`
/// clears it back to all-must-approve (the default barrier). `cancel_on_reject` is
/// left intact — the runtime merely ignores it while a quorum is set (DD7), so
/// toggling the quorum off restores the prior toggle. No-op on an unknown join id.
export function setJoinQuorum(d: DraftPipeline, joinId: string, quorum: number | undefined): DraftPipeline {
  return mapJoin(d, joinId, (j) => ({ ...j, quorum }));
}

/// Set a join's early-cancel-on-reject policy (AU1/P2). No-op on an unknown join id.
export function setJoinCancelOnReject(d: DraftPipeline, joinId: string, value: boolean): DraftPipeline {
  return mapJoin(d, joinId, (j) => ({ ...j, cancel_on_reject: value }));
}
