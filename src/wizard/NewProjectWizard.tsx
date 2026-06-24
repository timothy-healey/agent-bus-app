import type React from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { createProjectFromDraft, kickoffGenerate, listSeedTemplates, seedTemplate, type DraftPipeline, type SeedTemplateSummary, type Step } from "../ipc/pipeline";
import type { Project } from "../ipc/workspace";
import { emptyDraft, WIZARD_STEPS, type WizardStep } from "./draft";
import { ChatDraftPanel } from "./ChatDraftPanel";
import { TeamsStep } from "./TeamsStep";
import { PromptsStep } from "./PromptsStep";
import { WiringStep } from "./WiringStep";
import { ReviewStep } from "./ReviewStep";
import { Button } from "../components/ui/Button";
import { useModalA11y } from "../hooks/useModalA11y";

interface NewProjectWizardProps {
  open: boolean;
  onClose: () => void;
  onCreated: (p: Project) => void;
}

const STEP_TO_API: Record<"teams" | "prompts" | "wiring", Step> = {
  teams: "teams",
  prompts: "prompts",
  wiring: "wiring",
};

/// The 5-step new-project wizard (the only new-project path). Ephemeral
/// DesignSession state (D8): step, basics, draft, a per-open sessionId. Closing
/// discards everything.
export function NewProjectWizard({ open, onClose, onCreated }: NewProjectWizardProps) {
  const [step, setStep] = useState<WizardStep>("basics");
  const [name, setName] = useState("");
  const [root, setRoot] = useState("");
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
      go("teams");
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
      go("teams");
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
      const project = await createProjectFromDraft(name, root, { ...draft, name, description });
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
            <label style={lbl}>Root path<input aria-label="Root path" value={root} onChange={(e) => setRoot(e.target.value)} placeholder="~/projects/example" style={inp} /></label>
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

        {(step === "teams" || step === "prompts" || step === "wiring") && (
          <div style={{ height: 520 }}>
            <h3 style={{ fontSize: "var(--ts-md)", textTransform: "capitalize" }}>{step}</h3>
            {step === "wiring" ? (
              // Wiring's draft view is the read-only viewer; chat still drives edits.
              <ChatDraftPanel
                sessionId={sessionId}
                step={STEP_TO_API[step]}
                draft={draft}
                onDraftChange={setDraft}
                renderDraft={(d, onChange) => <WiringStep draft={d} onChange={onChange} />}
              />
            ) : (
              <ChatDraftPanel
                sessionId={sessionId}
                step={STEP_TO_API[step]}
                draft={draft}
                onDraftChange={setDraft}
                renderDraft={(d, onChange) => (step === "teams" ? <TeamsStep draft={d} onChange={onChange} /> : <PromptsStep draft={d} onChange={onChange} />)}
              />
            )}
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
              <Button onClick={() => go(step === "review" ? "wiring" : WIZARD_STEPS[idx - 1])}>Back</Button>
            )}
            {(step === "teams" || step === "prompts" || step === "wiring") && (
              <Button onClick={() => go(WIZARD_STEPS[idx + 1])}>Next</Button>
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
