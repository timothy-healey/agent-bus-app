import type React from "react";
import { useMemo, useState } from "react";
import { kickoffGenerate, type DraftPipeline, type Step } from "../ipc/pipeline";
import type { Project } from "../ipc/workspace";
import { emptyDraft, WIZARD_STEPS, type WizardStep } from "./draft";
import { ChatDraftPanel } from "./ChatDraftPanel";
import { TeamsStep } from "./TeamsStep";
import { PromptsStep } from "./PromptsStep";
import { WiringStep } from "./WiringStep";
import { ReviewStep } from "./ReviewStep";

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
  // a stable per-open dialogue session id (D8).
  const sessionId = useMemo(() => `wiz-${Math.random().toString(36).slice(2)}`, [open]);

  if (!open) return null;

  const idx = WIZARD_STEPS.indexOf(step);
  const go = (next: WizardStep) => setStep(next);

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

  return (
    <div role="dialog" aria-modal="true" style={overlay} onClick={onClose}>
      <div onClick={(e) => e.stopPropagation()} style={panel}>
        <div style={{ display: "flex", gap: 8, marginBottom: "var(--sp-4)" }}>
          {WIZARD_STEPS.map((s) => (
            <span key={s} style={{ color: s === step ? "var(--text)" : "var(--text-3)", fontSize: 12, textTransform: "capitalize" }}>{s}</span>
          ))}
        </div>

        {step === "basics" && (
          <div>
            <label style={lbl}>Project name<input aria-label="Project name" value={name} onChange={(e) => setName(e.target.value)} style={inp} /></label>
            <label style={lbl}>Root path<input aria-label="Root path" value={root} onChange={(e) => setRoot(e.target.value)} placeholder="~/projects/example" style={inp} /></label>
            <label style={lbl}>Describe what you're building<textarea aria-label="Describe what you're building" value={description} onChange={(e) => setDescription(e.target.value)} rows={4} style={inp} /></label>
            <button onClick={generate} disabled={busy || !name.trim() || !root.trim() || !description.trim()}>Generate</button>
          </div>
        )}

        {(step === "teams" || step === "prompts" || step === "wiring") && (
          <div style={{ height: 420 }}>
            <h3 style={{ fontSize: 13, textTransform: "capitalize" }}>{step}</h3>
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
          <ReviewStep basics={{ name, root, description }} draft={draft} onCreated={onCreated} />
        )}

        <div style={{ display: "flex", justifyContent: "space-between", marginTop: "var(--sp-5)" }}>
          <button onClick={onClose}>Cancel</button>
          <div style={{ display: "flex", gap: 8 }}>
            {idx > 0 && step !== "review" && <button onClick={() => go(WIZARD_STEPS[idx - 1])}>Back</button>}
            {step !== "basics" && step !== "review" && <button onClick={() => go(WIZARD_STEPS[idx + 1])}>Next</button>}
          </div>
        </div>
      </div>
    </div>
  );
}

const overlay: React.CSSProperties = { position: "fixed", inset: 0, background: "rgba(0,0,0,0.5)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 100 };
const panel: React.CSSProperties = { background: "var(--surface)", border: "1px solid var(--border)", borderRadius: "var(--r-md)", padding: "var(--sp-7)", minWidth: 720, maxWidth: 900 };
const lbl: React.CSSProperties = { display: "block", fontSize: 11, color: "var(--text-3)", marginBottom: "var(--sp-3)" };
const inp: React.CSSProperties = { width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", fontFamily: "inherit", fontSize: 12 };
