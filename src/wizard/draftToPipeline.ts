import type { DraftPipeline, Pipeline } from "../ipc/pipeline";

/// Adapt a DraftPipeline to the Pipeline shape the read-only viewer expects:
/// inline prompt bodies become placeholder paths; gates carry through (W3).
export function draftToPipeline(d: DraftPipeline): Pipeline {
  return {
    id: d.id || "(draft)",
    name: d.name || "(unnamed)",
    description: d.description,
    schema_version: d.schema_version,
    teams: d.teams.map((t) => ({ ...t, prompt: `prompts/${t.id}.md`, role: t.role, store: t.store })),
    gates: d.gates,
    escalations: d.escalations,
    forks: d.forks,
    joins: d.joins,
  };
}
