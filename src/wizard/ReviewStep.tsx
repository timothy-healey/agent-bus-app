import type { DraftPipeline } from "../ipc/pipeline";
import { PipelineView } from "../components/PipelineView";
import { draftToPipeline } from "./WiringStep";

interface ReviewStepProps {
  basics: { name: string; root: string; description: string };
  draft: DraftPipeline;
  error?: string | null;
}

/// Step 5: review the assembled pipeline + every prompt file. Presentational —
/// the Create action lives in the wizard footer (NewProjectWizard), which calls
/// create_project_from_draft (hard-validate → write → activate) and surfaces any
/// hard-validation failure back here as `error` (nothing is written on failure).
export function ReviewStep({ basics, draft, error }: ReviewStepProps) {
  const named = { ...draft, name: basics.name, description: basics.description };

  return (
    <div>
      <PipelineView pipeline={draftToPipeline(named)} />
      <h3 style={{ fontSize: 12, color: "var(--text-2)" }}>Prompt files</h3>
      {named.teams.map((t) => (
        <div key={t.id} style={{ marginBottom: "var(--sp-2)" }}>
          <div style={{ color: "var(--text-3)", fontSize: 11 }}>prompts/{t.id}.md</div>
          <pre style={{ whiteSpace: "pre-wrap", background: "var(--bg-2)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", color: "var(--text)" }}>{t.prompt_body}</pre>
        </div>
      ))}
      {error && (
        <div role="alert" style={{ color: "var(--danger)", fontSize: "var(--ts-base)", border: "1px solid var(--danger)", background: "var(--danger-2)", borderRadius: "var(--r-sm)", padding: "var(--sp-2)", marginBottom: "var(--sp-2)" }}>{error}</div>
      )}
    </div>
  );
}
