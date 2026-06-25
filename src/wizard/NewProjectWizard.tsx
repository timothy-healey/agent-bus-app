import type React from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { createProjectFromDraft, kickoffGenerate, listSeedTemplates, seedTemplate, type DraftPipeline, type SeedTemplateSummary } from "../ipc/pipeline";
import type { Project } from "../ipc/workspace";
import { emptyDraft, WIZARD_STEPS, type WizardStep } from "./draft";
import { PipelineCanvas } from "./PipelineCanvas";
import { ReviewStep } from "./ReviewStep";
import { AuthoringLayout, type NavStep } from "./AuthoringLayout";
import { ProjectSwitcher } from "../components/ProjectSwitcher";
import { Button } from "../components/ui/Button";
import { FolderPickerField } from "../components/FolderPickerField";
import { useSkillCatalog } from "../hooks/useSkillCatalog";

interface NewProjectWizardProps {
  open: boolean;
  onClose: () => void;
  onCreated: (p: Project) => void;
  /// The existing projects, for the left-nav switcher (G13/G14). Selecting one
  /// abandons the in-progress draft (with confirm) and switches to it.
  projects?: Project[];
  activeProjectId?: string | null;
  onSelectProject?: (id: string) => void;
  onDeleteProject?: (id: string) => void | Promise<void>;
}

const STEP_LABELS: Record<WizardStep, string> = { basics: "Basics", canvas: "Canvas", review: "Review" };

/// The new-project wizard (the only new-project path): Basics → Canvas → Review.
/// G10/G14: a full-page authoring view (AuthoringLayout) — left nav tree + the
/// project switcher beside a main pane hosting the current step — NOT a centered
/// modal. The Teams/Prompts/Wiring form trio + per-step Design Session chat are
/// replaced by the interactive PipelineCanvas (graph-builder); Basics "Generate"
/// / templates pre-fill the draft, which simply renders as an editable graph.
/// Ephemeral DesignSession state (D8): step, basics, draft, a per-open sessionId.
/// Closing discards everything.
export function NewProjectWizard({
  open,
  onClose,
  onCreated,
  projects = [],
  activeProjectId = null,
  onSelectProject,
  onDeleteProject,
}: NewProjectWizardProps) {
  const [step, setStep] = useState<WizardStep>("basics");
  const [name, setName] = useState("");
  const [root, setRoot] = useState("");
  const [targetRepo, setTargetRepo] = useState("");
  const [description, setDescription] = useState("");
  const [draft, setDraft] = useState<DraftPipeline>(emptyDraft());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // G11 — live best-effort validity reported by the canvas. The PROMINENT banner +
  // navigation block only kick in on a Continue/Create/leave-Canvas ATTEMPT (the
  // subtle inline badges carry validity the rest of the time).
  const [canvasValid, setCanvasValid] = useState(true);
  const [showBanner, setShowBanner] = useState(false);
  // a stable per-open dialogue session id (D8).
  const sessionId = useMemo(() => `wiz-${Math.random().toString(36).slice(2)}`, [open]);
  const [templates, setTemplates] = useState<SeedTemplateSummary[]>([]);

  // Load the bundled seed templates for the kickoff picker (A2).
  useEffect(() => {
    if (!open) return;
    let active = true;
    listSeedTemplates().then((t) => { if (active) setTemplates(t); }).catch(() => {});
    return () => { active = false; };
  }, [open]);

  // G4 — the GLOBAL skill catalog (no project exists yet mid-create). Passed to the
  // canvas so the prompt `/`-autocomplete works during creation, not just edit-mode.
  const { entries: skills, refresh: refreshSkills } = useSkillCatalog(null);

  const idx = WIZARD_STEPS.indexOf(step);
  const go = (next: WizardStep) => setStep(next);

  // G11 — attempt to move forward off the Canvas (to Review or Create). If the
  // draft is invalid, reveal the prominent banner and BLOCK; otherwise proceed.
  // Moving backward (to Basics) or staying never blocks. Returns whether allowed.
  const attemptForwardFromCanvas = useCallback(
    (target: WizardStep): boolean => {
      // Only forward moves off the canvas are gated.
      const movingForward = WIZARD_STEPS.indexOf(target) > WIZARD_STEPS.indexOf("canvas");
      if (step === "canvas" && movingForward && !canvasValid) {
        setShowBanner(true);
        return false;
      }
      return true;
    },
    [step, canvasValid],
  );

  // Leaving (Cancel / Escape / switching project) discards all wizard state, so
  // confirm when the draft is dirty (audit A3). Dirty = past the first step, or
  // any basics typed.
  const dirty =
    step !== "basics" || name.trim() !== "" || root.trim() !== "" || description.trim() !== "";
  const confirmLeave = useCallback(
    () => !dirty || window.confirm("Discard this draft project? Your changes will be lost."),
    [dirty],
  );
  const requestClose = useCallback(() => {
    if (confirmLeave()) onClose();
  }, [confirmLeave, onClose]);

  if (!open) return null;

  // G12 — a draft exists once the canvas has any node (generated / seeded /
  // hand-authored). When it does, Basics offers a draft-preserving Continue and
  // Generate becomes a confirm-gated Regenerate, so the authored draft is never
  // silently clobbered.
  const hasDraft =
    draft.teams.length > 0 ||
    draft.gates.length > 0 ||
    draft.forks.length > 0 ||
    draft.joins.length > 0 ||
    draft.escalations.length > 0;

  async function generate() {
    // G12 — Regenerate must never silently replace an authored draft.
    if (hasDraft && !window.confirm("Replace the current draft? Your authored graph will be overwritten.")) {
      return;
    }
    setBusy(true);
    try {
      const d = await kickoffGenerate(sessionId, description);
      setDraft({ ...d, name, description });
      go("canvas");
    } finally {
      setBusy(false);
    }
  }

  // Start from a bundled template (A2): seed the draft, then refine it through the
  // normal wizard steps. Both kickoff paths end in the same editable draft.
  async function startFromTemplate(id: string) {
    setBusy(true);
    try {
      const d = await seedTemplate(id);
      // Carry the user's name (description optional when seeding from a template).
      setDraft({ ...d, name, description: description.trim() || d.description });
      go("canvas");
    } finally {
      setBusy(false);
    }
  }

  // Step 3 Create: hard-validate → write → activate via create_project_from_draft.
  // A hard-validation failure comes back as an error and nothing is written.
  async function create() {
    // G11 — block + reveal the banner on a known-invalid draft rather than letting
    // the backend reject it (the backend stays the hard authority either way).
    if (!canvasValid) {
      setShowBanner(true);
      setStep("canvas");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const project = await createProjectFromDraft(name, root, { ...draft, name, description }, targetRepo.trim() || null);
      onCreated(project as Project);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const steps: NavStep[] = WIZARD_STEPS.map((s) => ({
    id: s,
    label: STEP_LABELS[s],
    // G11 — the nav tree is free EXCEPT moving forward off an invalid Canvas, which
    // reveals the banner + blocks (attempt-time gating, not an always-on disable).
    // G12 — Basics→Canvas is draft-preserving (a plain go, never a re-Generate).
    onSelect: () => { if (attemptForwardFromCanvas(s)) go(s); },
  }));

  // Switching to an existing project abandons the draft (confirm first).
  const handleSelectProject = onSelectProject
    ? (id: string) => { if (confirmLeave()) onSelectProject(id); }
    : undefined;

  const switcher = (
    <ProjectSwitcher
      projects={projects}
      activeProjectId={activeProjectId}
      onSelect={handleSelectProject}
      onDelete={onDeleteProject}
    />
  );

  const footer = (
    <>
      <Button variant="ghost" onClick={requestClose}>Cancel</Button>
      {step !== "basics" && (
        <Button onClick={() => go(WIZARD_STEPS[idx - 1])}>Back</Button>
      )}
      {step === "canvas" && (
        <Button variant="primary" onClick={() => { if (attemptForwardFromCanvas("review")) go("review"); }}>Continue</Button>
      )}
      {step === "review" && (
        <Button variant="primary" onClick={create} disabled={busy}>
          {busy ? "Creating…" : "Create project"}
        </Button>
      )}
    </>
  );

  return (
    <AuthoringLayout
      title="New project"
      steps={steps}
      currentStepId={step}
      switcher={switcher}
      footer={footer}
      onClose={requestClose}
    >
      {step === "basics" && (
        <div style={{ overflowY: "auto", minHeight: 0 }}>
          <label style={lbl}>Project name<input aria-label="Project name" value={name} onChange={(e) => setName(e.target.value)} style={inp} /></label>
          <div style={lbl}><FolderPickerField label="Root path" value={root} onChange={setRoot} placeholder="~/projects/example" /></div>
          <div style={lbl}><FolderPickerField label="Target repo (optional)" value={targetRepo} onChange={setTargetRepo} placeholder="~/projects/your-repo" /></div>
          <label style={lbl}>Describe what you're building<textarea aria-label="Describe what you're building" value={description} onChange={(e) => setDescription(e.target.value)} rows={4} style={inp} /></label>
          <div style={{ display: "flex", gap: "var(--sp-2)", flexWrap: "wrap" }}>
            {/* G12 — when a draft exists, Continue is the draft-preserving path and
                Generate becomes a confirm-gated Regenerate (in `generate`). */}
            {hasDraft && (
              <Button variant="primary" onClick={() => go("canvas")} aria-label="continue to canvas">
                Continue →
              </Button>
            )}
            <Button variant={hasDraft ? "default" : "primary"} onClick={generate} disabled={busy || !name.trim() || !root.trim() || !description.trim()}>
              {busy ? "Generating…" : hasDraft ? "Regenerate" : "Generate"}
            </Button>
          </div>
          {templates.length > 0 && (
            <div style={{ marginTop: "var(--sp-4)" }}>
              <div style={{ fontSize: "var(--ts-sm)", color: "var(--text-3)", marginBottom: "var(--sp-2)" }}>Or start from a template</div>
              <div style={{ display: "flex", flexWrap: "wrap", gap: "var(--sp-2)" }}>
                {templates.map((t) => (
                  <Button
                    key={t.id}
                    title={t.description}
                    onClick={() => startFromTemplate(t.id)}
                    disabled={busy || !name.trim() || !root.trim()}
                  >
                    {t.name}
                  </Button>
                ))}
              </div>
            </div>
          )}
        </div>
      )}

      {step === "canvas" && (
        <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}>
          <PipelineCanvas
            draft={draft}
            onChange={setDraft}
            skills={skills}
            onRefreshSkills={refreshSkills}
            showBanner={showBanner}
            targetRepo={targetRepo.trim() || null}
            onValidityChange={(valid) => {
              setCanvasValid(valid);
              // Once the draft is fixed, retract the prominent banner (it returns on
              // the next blocked attempt).
              if (valid) setShowBanner(false);
            }}
          />
        </div>
      )}

      {step === "review" && (
        <div style={{ overflowY: "auto", minHeight: 0 }}>
          <ReviewStep basics={{ name, root, description }} draft={draft} error={error} />
        </div>
      )}
    </AuthoringLayout>
  );
}

const lbl: React.CSSProperties = { display: "block", fontSize: "var(--ts-sm)", color: "var(--text-3)", marginBottom: "var(--sp-3)" };
const inp: React.CSSProperties = { width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", fontFamily: "inherit", fontSize: "var(--ts-base)" };
