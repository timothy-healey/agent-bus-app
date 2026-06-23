import type React from "react";
import { useState } from "react";
import { bestEffortValidate, savePipelineEdits, type DraftPipeline } from "../ipc/pipeline";
import { TeamsStep } from "./TeamsStep";
import { PromptsStep } from "./PromptsStep";
import { WiringStep } from "./WiringStep";

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
    <div role="dialog" aria-modal="true" aria-label="Edit pipeline" style={overlay} onClick={onClose}>
      <div onClick={(e) => e.stopPropagation()} style={panel}>
        <div style={{ display: "flex", gap: 8, marginBottom: "var(--sp-4)" }}>
          {EDIT_STEPS.map((s) => (
            <button
              key={s}
              onClick={() => setStep(s)}
              style={{
                background: s === step ? "var(--surface-2)" : "transparent",
                border: "1px solid",
                borderColor: s === step ? "var(--border-2)" : "transparent",
                color: s === step ? "var(--text)" : "var(--text-3)",
                padding: "3px 12px",
                borderRadius: "var(--r-sm)",
                fontSize: 12,
                textTransform: "capitalize",
                cursor: "pointer",
              }}
            >
              {s}
            </button>
          ))}
        </div>

        {issues.length > 0 && (
          <div
            role="status"
            aria-label="validation issues"
            style={{ marginBottom: "var(--sp-3)", padding: "var(--sp-2)", border: "1px solid var(--accent-bd)", background: "var(--accent-2)", borderRadius: "var(--r-sm)", color: "var(--text-2)", fontSize: 11 }}
          >
            {issues.map((iss, i) => (
              <div key={i}>• {iss}</div>
            ))}
          </div>
        )}

        <div style={{ minHeight: 360, maxHeight: 460, overflowY: "auto" }}>
          {step === "teams" && <TeamsStep draft={draft} onChange={onChange} />}
          {step === "prompts" && <PromptsStep draft={draft} onChange={onChange} />}
          {step === "wiring" && <WiringStep draft={draft} onChange={onChange} />}
        </div>

        {error && <div style={{ color: "var(--danger)", fontSize: 12, marginTop: "var(--sp-2)" }}>{error}</div>}

        <div style={{ display: "flex", justifyContent: "space-between", marginTop: "var(--sp-5)" }}>
          <button onClick={onClose}>Cancel</button>
          <div style={{ display: "flex", gap: 8 }}>
            {idx > 0 && <button onClick={() => setStep(EDIT_STEPS[idx - 1])}>Back</button>}
            {idx < EDIT_STEPS.length - 1 && <button onClick={() => setStep(EDIT_STEPS[idx + 1])}>Next</button>}
            <button onClick={save} disabled={busy} aria-label="save pipeline">Save pipeline</button>
          </div>
        </div>
      </div>
    </div>
  );
}

const overlay: React.CSSProperties = { position: "fixed", inset: 0, background: "rgba(0,0,0,0.5)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 100 };
const panel: React.CSSProperties = { background: "var(--surface)", border: "1px solid var(--border)", borderRadius: "var(--r-md)", padding: "var(--sp-7)", minWidth: 720, maxWidth: 900 };
