import type React from "react";
import { useState } from "react";
import { bestEffortValidate, savePipelineEdits, type DraftPipeline } from "../ipc/pipeline";
import { TeamsStep } from "./TeamsStep";
import { PromptsStep } from "./PromptsStep";
import { WiringStep } from "./WiringStep";
import { Button } from "../components/ui/Button";
import { useModalA11y } from "../hooks/useModalA11y";

const EDIT_STEPS = ["teams", "prompts", "wiring"] as const;
type EditStep = (typeof EDIT_STEPS)[number];

interface PipelineEditorProps {
  projectId: string;
  seed: DraftPipeline;
  onClose: () => void;
  onSaved: () => void;
}

/// In-app pipeline editor (A1). REUSES the wizard's step editors over a local
/// draft seeded from the active pipeline. This is "Pipeline edit-mode": a manual
/// DraftPipeline edit — NOT a Design Session (no chat / no LLM Chat ACL). Save
/// hard-validates backend-side and overwrites the YAML + prompt files.
export function PipelineEditor({ projectId, seed, onClose, onSaved }: PipelineEditorProps) {
  const [draft, setDraft] = useState<DraftPipeline>(seed);
  const [step, setStep] = useState<EditStep>("teams");
  const [issues, setIssues] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const idx = EDIT_STEPS.indexOf(step);
  // Focus trap + Escape-to-close + restore-focus (audit A2).
  const dialogRef = useModalA11y<HTMLDivElement>(true, onClose);

  // Manual edits re-fetch the backend's best-effort issues (the backend stays the
  // validation authority — same contract as ChatDraftPanel's manual-edit path).
  function onChange(d: DraftPipeline) {
    setDraft(d);
    bestEffortValidate(d).then(setIssues).catch(() => {});
  }

  async function save() {
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

  return (
    <div style={overlay} onClick={onClose}>
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label="Edit pipeline"
        onClick={(e) => e.stopPropagation()}
        style={panel}
      >
        <div style={{ display: "flex", gap: 8, marginBottom: "var(--sp-4)" }}>
          {EDIT_STEPS.map((s) => (
            <button
              key={s}
              className="abp-tab"
              onClick={() => setStep(s)}
              style={{
                background: s === step ? "var(--surface-2)" : "transparent",
                border: "1px solid",
                borderColor: s === step ? "var(--border-2)" : "transparent",
                color: s === step ? "var(--text)" : "var(--text-3)",
                padding: "3px 12px",
                borderRadius: "var(--r-sm)",
                fontSize: "var(--ts-base)",
                textTransform: "capitalize",
                cursor: "pointer",
                fontFamily: "inherit",
              }}
            >
              {s}
            </button>
          ))}
        </div>

        {issues.length > 0 && (
          <ul
            role="status"
            aria-label="validation issues"
            style={{ listStyle: "none", margin: "0 0 var(--sp-3)", padding: "var(--sp-2)", border: "1px solid var(--warn)", background: "var(--warn-2)", borderRadius: "var(--r-sm)", color: "var(--text-2)", fontSize: "var(--ts-sm)" }}
          >
            {issues.map((iss, i) => (
              <li key={i}>• {iss}</li>
            ))}
          </ul>
        )}

        <div style={{ minHeight: 360, maxHeight: 460, overflowY: "auto" }}>
          {step === "teams" && <TeamsStep draft={draft} onChange={onChange} />}
          {step === "prompts" && <PromptsStep draft={draft} onChange={onChange} />}
          {step === "wiring" && <WiringStep draft={draft} onChange={onChange} />}
        </div>

        {error && (
          <div role="alert" style={{ color: "var(--danger)", fontSize: "var(--ts-base)", marginTop: "var(--sp-2)", border: "1px solid var(--danger)", background: "var(--danger-2)", borderRadius: "var(--r-sm)", padding: "var(--sp-2)" }}>{error}</div>
        )}

        <div style={{ display: "flex", justifyContent: "space-between", marginTop: "var(--sp-5)" }}>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <div style={{ display: "flex", gap: 8 }}>
            {idx > 0 && <Button onClick={() => setStep(EDIT_STEPS[idx - 1])}>Back</Button>}
            {idx < EDIT_STEPS.length - 1 && <Button onClick={() => setStep(EDIT_STEPS[idx + 1])}>Next</Button>}
            <Button variant="primary" onClick={save} disabled={busy} aria-label="save pipeline">
              {busy ? "Saving…" : "Save pipeline"}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

const overlay: React.CSSProperties = { position: "fixed", inset: 0, background: "var(--scrim)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 100 };
const panel: React.CSSProperties = { background: "var(--surface)", border: "1px solid var(--border)", borderRadius: "var(--r-md)", padding: "var(--sp-7)", minWidth: 720, maxWidth: 900 };
