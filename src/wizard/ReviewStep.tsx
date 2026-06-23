import { useState } from "react";
import { createProjectFromDraft, type DraftPipeline } from "../ipc/pipeline";
import type { Project } from "../ipc/workspace";
import { PipelineView } from "../components/PipelineView";
import { draftToPipeline } from "./WiringStep";

interface ReviewStepProps {
  basics: { name: string; root: string; description: string };
  draft: DraftPipeline;
  onCreated: (p: Project) => void;
}

/// Step 5: review the assembled pipeline + every prompt file, then Create.
/// Create calls create_project_from_draft (hard-validate → write → activate);
/// a hard-validation failure comes back as an error and nothing is written.
export function ReviewStep({ basics, draft, onCreated }: ReviewStepProps) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const named = { ...draft, name: basics.name, description: basics.description };

  async function create() {
    setBusy(true);
    setError(null);
    try {
      const project = await createProjectFromDraft(basics.name, basics.root, named);
      onCreated(project as Project);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

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
      {error && <div style={{ color: "var(--danger)", fontSize: 12 }}>{error}</div>}
      <button onClick={create} disabled={busy}>Create project</button>
    </div>
  );
}
