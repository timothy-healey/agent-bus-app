import type { DraftPipeline, DraftTeam } from "../ipc/pipeline";

export const WIZARD_STEPS = ["basics", "teams", "prompts", "wiring", "review"] as const;
export type WizardStep = (typeof WIZARD_STEPS)[number];

const SCHEMA_VERSION = 2;

export function emptyDraft(): DraftPipeline {
  return {
    id: "",
    name: "",
    description: "",
    schema_version: SCHEMA_VERSION,
    teams: [],
    forks: [],
    joins: [],
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
    workers: { default: 1, max: 1 },
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
