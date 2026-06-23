import { useState } from "react";
import type { DraftPipeline } from "../ipc/pipeline";
import { removeTeam, renameTeam, setTeamModel } from "./draft";

interface TeamsStepProps {
  draft: DraftPipeline;
  onChange: (d: DraftPipeline) => void;
}

/// Step 2 draft view: team cards with rename/remove + a per-team advanced panel
/// (model in v1; effort/tools/scope follow the same controlled-input pattern).
export function TeamsStep({ draft, onChange }: TeamsStepProps) {
  const [openAdvanced, setOpenAdvanced] = useState<string | null>(null);

  return (
    <div>
      {draft.teams.map((t) => (
        <div key={t.id} style={{ border: "1px solid var(--border)", borderRadius: "var(--r-sm)", padding: "var(--sp-3)", marginBottom: "var(--sp-2)" }}>
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <input
              aria-label={`name for ${t.id}`}
              value={t.name}
              onChange={(e) => onChange(renameTeam(draft, t.id, e.target.value))}
              style={{ flex: 1, background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)" }}
            />
            <span style={{ color: "var(--text-3)", fontSize: 11 }}>{t.id}</span>
            <button aria-label={`advanced ${t.id}`} onClick={() => setOpenAdvanced((o) => (o === t.id ? null : t.id))}>⚙</button>
            <button aria-label={`remove ${t.id}`} onClick={() => onChange(removeTeam(draft, t.id))}>✕</button>
          </div>
          {openAdvanced === t.id && (
            <div style={{ marginTop: 6 }}>
              <label style={{ fontSize: 11, color: "var(--text-3)" }}>
                model
                <input
                  aria-label={`model for ${t.id}`}
                  value={t.runner.model}
                  onChange={(e) => onChange(setTeamModel(draft, t.id, e.target.value))}
                  style={{ width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)" }}
                />
              </label>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
