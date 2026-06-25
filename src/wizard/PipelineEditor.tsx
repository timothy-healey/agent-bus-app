import type React from "react";
import { useState } from "react";
import { savePipelineEdits, type DraftPipeline } from "../ipc/pipeline";
import type { Project } from "../ipc/workspace";
import { PipelineCanvas } from "./PipelineCanvas";
import { ReviewStep } from "./ReviewStep";
import { AuthoringLayout, type NavStep } from "./AuthoringLayout";
import { ProjectSwitcher } from "../components/ProjectSwitcher";
import { Button } from "../components/ui/Button";
import { useSkillCatalog } from "../hooks/useSkillCatalog";

type EditorStep = "canvas" | "review";

interface PipelineEditorProps {
  projectId: string;
  seed: DraftPipeline;
  onClose: () => void;
  onSaved: () => void;
  /// The existing projects, for the left-nav switcher (G13/G14).
  projects?: Project[];
  onSelectProject?: (id: string) => void;
  onDeleteProject?: (id: string) => void | Promise<void>;
}

const STEP_LABELS: Record<EditorStep, string> = { canvas: "Canvas", review: "Review" };
const EDITOR_STEPS: EditorStep[] = ["canvas", "review"];

/// In-app pipeline editor (A1). Renders the shared PipelineCanvas over a local
/// draft seeded from the active pipeline — the SAME interactive surface as the
/// new-project wizard, now in the SAME full-page AuthoringLayout (G10). This is
/// "Pipeline edit-mode": a manual DraftPipeline edit (NOT a Design Session — no
/// chat / no LLM Chat ACL). Save hard-validates backend-side and overwrites the
/// YAML + prompt files. The canvas runs live best-effort validation itself.
export function PipelineEditor({
  projectId,
  seed,
  onClose,
  onSaved,
  projects = [],
  onSelectProject,
  onDeleteProject,
}: PipelineEditorProps) {
  const [draft, setDraft] = useState<DraftPipeline>(seed);
  const [step, setStep] = useState<EditorStep>("canvas");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // G11 — live validity from the canvas; the prominent banner + Save block only on
  // a Save/leave-Canvas attempt.
  const [canvasValid, setCanvasValid] = useState(true);
  const [showBanner, setShowBanner] = useState(false);

  // A4: the skill catalog for this project, feeding NodeDrawer's prompt
  // autocomplete. Loaded at open + manually refreshable.
  const { entries: skills, refresh: refreshSkills } = useSkillCatalog(projectId);

  async function save() {
    // G11 — block + surface the banner on a known-invalid draft (the backend stays
    // the hard authority; this avoids the round-trip + raises the offending badges).
    if (!canvasValid) {
      setShowBanner(true);
      setStep("canvas");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await savePipelineEdits(projectId, draft);
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const steps: NavStep[] = EDITOR_STEPS.map((s) => ({
    id: s,
    label: STEP_LABELS[s],
    // G11 — leaving an invalid Canvas (forward, to Review) reveals the banner +
    // blocks; moving back to Canvas never blocks.
    onSelect: () => {
      if (step === "canvas" && s === "review" && !canvasValid) {
        setShowBanner(true);
        return;
      }
      setStep(s);
    },
  }));

  const switcher = (
    <ProjectSwitcher
      projects={projects}
      activeProjectId={projectId}
      onSelect={onSelectProject}
      onDelete={onDeleteProject}
    />
  );

  const footer = (
    <>
      <Button variant="ghost" onClick={onClose}>Cancel</Button>
      <Button variant="primary" onClick={save} disabled={busy} aria-label="save pipeline">
        {busy ? "Saving…" : "Save pipeline"}
      </Button>
    </>
  );

  return (
    <AuthoringLayout
      title="Edit pipeline"
      steps={steps}
      currentStepId={step}
      switcher={switcher}
      footer={footer}
      onClose={onClose}
    >
      {step === "canvas" && (
        <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
          <PipelineCanvas
            draft={draft}
            onChange={setDraft}
            skills={skills}
            onRefreshSkills={refreshSkills}
            showBanner={showBanner}
            targetRepo={projects.find((p) => p.id === projectId)?.target_repo ?? null}
            onValidityChange={(valid) => { setCanvasValid(valid); if (valid) setShowBanner(false); }}
          />
        </div>
      )}

      {step === "review" && (
        <div style={{ overflowY: "auto", minHeight: 0 }}>
          <ReviewStep basics={{ name: draft.name, root: "", description: draft.description }} draft={draft} />
        </div>
      )}

      {error && (
        <div role="alert" style={alert}>{error}</div>
      )}
    </AuthoringLayout>
  );
}

const alert: React.CSSProperties = { color: "var(--danger)", fontSize: "var(--ts-base)", marginTop: "var(--sp-2)", border: "1px solid var(--danger)", background: "var(--danger-2)", borderRadius: "var(--r-sm)", padding: "var(--sp-2)" };
