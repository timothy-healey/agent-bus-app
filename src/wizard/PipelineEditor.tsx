import type React from "react";
import { useState } from "react";
import { savePipelineEdits, type DraftPipeline } from "../ipc/pipeline";
import { PipelineCanvas } from "./PipelineCanvas";
import { Button } from "../components/ui/Button";
import { useModalA11y } from "../hooks/useModalA11y";

interface PipelineEditorProps {
  projectId: string;
  seed: DraftPipeline;
  onClose: () => void;
  onSaved: () => void;
}

/// In-app pipeline editor (A1). Renders the shared PipelineCanvas over a local
/// draft seeded from the active pipeline — the SAME interactive surface as the
/// new-project wizard. This is "Pipeline edit-mode": a manual DraftPipeline edit
/// (NOT a Design Session — no chat / no LLM Chat ACL). Save hard-validates
/// backend-side and overwrites the YAML + prompt files. The canvas runs live
/// best-effort validation itself.
export function PipelineEditor({ projectId, seed, onClose, onSaved }: PipelineEditorProps) {
  const [draft, setDraft] = useState<DraftPipeline>(seed);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Focus trap + Escape-to-close + restore-focus (audit A2).
  const dialogRef = useModalA11y<HTMLDivElement>(true, onClose);

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
        <h2 style={{ fontSize: "var(--ts-lg)", color: "var(--text)", margin: "0 0 var(--sp-3)" }}>Edit pipeline</h2>

        <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
          <PipelineCanvas draft={draft} onChange={setDraft} />
        </div>

        {error && (
          <div role="alert" style={{ color: "var(--danger)", fontSize: "var(--ts-base)", marginTop: "var(--sp-2)", border: "1px solid var(--danger)", background: "var(--danger-2)", borderRadius: "var(--r-sm)", padding: "var(--sp-2)" }}>{error}</div>
        )}

        <div style={{ display: "flex", justifyContent: "space-between", marginTop: "var(--sp-5)" }}>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" onClick={save} disabled={busy} aria-label="save pipeline">
            {busy ? "Saving…" : "Save pipeline"}
          </Button>
        </div>
      </div>
    </div>
  );
}

const overlay: React.CSSProperties = { position: "fixed", inset: 0, background: "var(--scrim)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 100 };
const panel: React.CSSProperties = { background: "var(--surface)", border: "1px solid var(--border)", borderRadius: "var(--r-md)", padding: "var(--sp-7)", width: "min(1100px, 94vw)", height: "90vh", display: "flex", flexDirection: "column", overflow: "hidden" };
