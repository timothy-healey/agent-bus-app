import type React from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { createProjectFromDraft, kickoffGenerate, listSeedTemplates, seedTemplate, type DraftPipeline, type SeedTemplateSummary } from "../ipc/pipeline";
import type { Project } from "../ipc/workspace";
import { emptyDraft, WIZARD_STEPS, type WizardStep } from "./draft";
import { PipelineCanvas } from "./PipelineCanvas";
import { ReviewStep } from "./ReviewStep";
import { Button } from "../components/ui/Button";
import { FolderPickerField } from "../components/FolderPickerField";
import { useModalA11y } from "../hooks/useModalA11y";

interface NewProjectWizardProps {
  open: boolean;
  onClose: () => void;
  onCreated: (p: Project) => void;
}

/// The new-project wizard (the only new-project path): Basics → Canvas → Review.
/// The Teams/Prompts/Wiring form trio + per-step Design Session chat are replaced
/// by the interactive PipelineCanvas (graph-builder); Basics "Generate" / templates
/// pre-fill the draft, which simply renders as an editable graph. Ephemeral
/// DesignSession state (D8): step, basics, draft, a per-open sessionId. Closing
/// discards everything.
export function NewProjectWizard({ open, onClose, onCreated }: NewProjectWizardProps) {
  const [step, setStep] = useState<WizardStep>("basics");
  const [name, setName] = useState("");
  const [root, setRoot] = useState("");
  const [targetRepo, setTargetRepo] = useState("");
  const [description, setDescription] = useState("");
  const [draft, setDraft] = useState<DraftPipeline>(emptyDraft());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
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

  const idx = WIZARD_STEPS.indexOf(step);
  const go = (next: WizardStep) => setStep(next);

  // Overlay-click / Escape discards all wizard state, so confirm when the draft
  // is dirty (audit A3). Dirty = past the first step, or any basics typed.
  const dirty =
    step !== "basics" || name.trim() !== "" || root.trim() !== "" || description.trim() !== "";
  const requestClose = useCallback(() => {
    if (!dirty || window.confirm("Discard this draft project? Your changes will be lost.")) {
      onClose();
    }
  }, [dirty, onClose]);

  // Focus trap + Escape + restore-focus (audit A3). Escape routes through the
  // dirty-confirm close.
  const dialogRef = useModalA11y<HTMLDivElement>(open, requestClose);

  if (!open) return null;

  async function generate() {
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

  // Step 5 Create: hard-validate → write → activate via create_project_from_draft.
  // A hard-validation failure comes back as an error and nothing is written.
  async function create() {
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

  return (
    <div style={overlay} onClick={requestClose}>
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label="New project"
        onClick={(e) => e.stopPropagation()}
        style={panel}
      >
        <div style={{ display: "flex", gap: 8, marginBottom: "var(--sp-4)" }}>
          {WIZARD_STEPS.map((s) => (
            <span key={s} style={{ color: s === step ? "var(--text)" : "var(--text-3)", fontSize: "var(--ts-base)", textTransform: "capitalize" }}>{s}</span>
          ))}
        </div>

        <div style={body}>
        {step === "basics" && (
          <div>
            <label style={lbl}>Project name<input aria-label="Project name" value={name} onChange={(e) => setName(e.target.value)} style={inp} /></label>
            <div style={lbl}><FolderPickerField label="Root path" value={root} onChange={setRoot} placeholder="~/projects/example" /></div>
            <div style={lbl}><FolderPickerField label="Target repo (optional)" value={targetRepo} onChange={setTargetRepo} placeholder="~/projects/your-repo" /></div>
            <label style={lbl}>Describe what you're building<textarea aria-label="Describe what you're building" value={description} onChange={(e) => setDescription(e.target.value)} rows={4} style={inp} /></label>
            <Button variant="primary" onClick={generate} disabled={busy || !name.trim() || !root.trim() || !description.trim()}>
              {busy ? "Generating…" : "Generate"}
            </Button>
            {templates.length > 0 && (
              <div style={{ marginTop: "var(--sp-4)" }}>
                <div style={{ fontSize: "var(--ts-sm)", color: "var(--text-3)", marginBottom: "var(--sp-2)" }}>Or start from a template</div>
                <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
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
          <div style={{ height: 560, display: "flex", flexDirection: "column", minHeight: 0 }}>
            <PipelineCanvas draft={draft} onChange={setDraft} />
          </div>
        )}

        {step === "review" && (
          <ReviewStep basics={{ name, root, description }} draft={draft} error={error} />
        )}
        </div>

        <div style={footer}>
          <Button variant="ghost" onClick={requestClose}>Cancel</Button>
          <div style={{ display: "flex", gap: 8 }}>
            {step !== "basics" && (
              <Button onClick={() => go(WIZARD_STEPS[idx - 1])}>Back</Button>
            )}
            {step === "canvas" && (
              <Button onClick={() => go("review")}>Next</Button>
            )}
            {step === "review" && (
              <Button variant="primary" onClick={create} disabled={busy}>
                {busy ? "Creating…" : "Create project"}
              </Button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

const overlay: React.CSSProperties = { position: "fixed", inset: 0, background: "var(--scrim)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 100 };
const panel: React.CSSProperties = { background: "var(--surface)", border: "1px solid var(--border)", borderRadius: "var(--r-md)", padding: "var(--sp-7)", width: "min(1100px, 94vw)", height: "92vh", display: "flex", flexDirection: "column", overflow: "hidden" };
const body: React.CSSProperties = { flex: 1, overflowY: "auto", minHeight: 0 };
const footer: React.CSSProperties = { display: "flex", justifyContent: "space-between", marginTop: "var(--sp-5)" };
const lbl: React.CSSProperties = { display: "block", fontSize: "var(--ts-sm)", color: "var(--text-3)", marginBottom: "var(--sp-3)" };
const inp: React.CSSProperties = { width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", fontFamily: "inherit", fontSize: 12 };
