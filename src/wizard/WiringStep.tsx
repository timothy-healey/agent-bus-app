import type { DraftPipeline, Pipeline } from "../ipc/pipeline";
import { PipelineView } from "../components/PipelineView";

/// Adapt a DraftPipeline to the Pipeline shape the read-only viewer expects:
/// inline prompt bodies become placeholder paths; gates carry through (W3).
export function draftToPipeline(d: DraftPipeline): Pipeline {
  return {
    id: d.id || "(draft)",
    name: d.name || "(unnamed)",
    description: d.description,
    schema_version: d.schema_version,
    teams: d.teams.map((t) => ({ ...t, prompt: `prompts/${t.id}.md` })),
    gates: d.gates,
    escalations: d.escalations,
    forks: d.forks,
    joins: d.joins,
  };
}

interface WiringStepProps {
  draft: DraftPipeline;
}

/// Step 4 draft view: the fork/join flow rendered through PipelineView.
export function WiringStep({ draft }: WiringStepProps) {
  return <PipelineView pipeline={draftToPipeline(draft)} />;
}
