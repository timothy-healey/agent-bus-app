import { useState } from "react";
import type React from "react";
import type { DraftPipeline, EffortMode } from "../ipc/pipeline";
import { removeTeam, renameTeam, setTeamModel, setTeamEffort, setTeamTools, setTeamReads, setTeamWrites } from "./draft";
import { Button } from "../components/ui/Button";

interface TeamsStepProps {
  draft: DraftPipeline;
  onChange: (d: DraftPipeline) => void;
}

const EFFORT_PRESETS: EffortMode["mode"][] = ["off", "standard", "extended-low", "extended-high", "custom"];

function effortFromSelect(mode: EffortMode["mode"], currentBudget: number): EffortMode {
  return mode === "custom" ? { mode: "custom", budget_tokens: currentBudget } : { mode };
}

/// Step 2 draft view: team cards with rename/remove + a per-team advanced panel
/// (model in v1; effort/tools/scope follow the same controlled-input pattern).
export function TeamsStep({ draft, onChange }: TeamsStepProps) {
  const [openAdvanced, setOpenAdvanced] = useState<string | null>(null);

  return (
    <div>
      {draft.teams.length === 0 && (
        <div style={{ padding: "var(--sp-5)", color: "var(--text-3)", fontSize: "var(--ts-base)", border: "1px dashed var(--border)", borderRadius: "var(--r-sm)" }}>
          no teams yet. describe what you're building in the chat to generate teams, or start from a template.
        </div>
      )}
      {draft.teams.map((t) => (
        <div key={t.id} style={{ border: "1px solid var(--border)", borderRadius: "var(--r-sm)", padding: "var(--sp-3)", marginBottom: "var(--sp-2)" }}>
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <input
              aria-label={`name for ${t.id}`}
              value={t.name}
              onChange={(e) => onChange(renameTeam(draft, t.id, e.target.value))}
              style={{ flex: 1, background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", fontFamily: "inherit" }}
            />
            <span style={{ color: "var(--text-3)", fontSize: "var(--ts-sm)" }}>{t.id}</span>
            <Button size="sm" variant="ghost" aria-label={`advanced ${t.id}`} aria-expanded={openAdvanced === t.id} onClick={() => setOpenAdvanced((o) => (o === t.id ? null : t.id))}>⚙</Button>
            <Button size="sm" variant="ghost" aria-label={`remove ${t.id}`} onClick={() => onChange(removeTeam(draft, t.id))}>✕</Button>
          </div>
          {openAdvanced === t.id && (
            <div style={{ marginTop: 6, display: "grid", gap: 6 }}>
              <label style={advLbl}>
                model
                <input
                  aria-label={`model for ${t.id}`}
                  value={t.runner.model}
                  onChange={(e) => onChange(setTeamModel(draft, t.id, e.target.value))}
                  style={advInp}
                />
              </label>
              <label style={advLbl}>
                effort
                <select
                  aria-label={`effort for ${t.id}`}
                  value={t.runner.effort.mode}
                  onChange={(e) =>
                    onChange(
                      setTeamEffort(
                        draft,
                        t.id,
                        effortFromSelect(
                          e.target.value as EffortMode["mode"],
                          t.runner.effort.mode === "custom" ? t.runner.effort.budget_tokens : 16000,
                        ),
                      ),
                    )
                  }
                  style={advInp}
                >
                  {EFFORT_PRESETS.map((m) => (
                    <option key={m} value={m}>{m}</option>
                  ))}
                </select>
              </label>
              {t.runner.effort.mode === "custom" && (
                <label style={advLbl}>
                  budget (tokens)
                  <input
                    type="number"
                    aria-label={`budget for ${t.id}`}
                    value={t.runner.effort.budget_tokens}
                    onChange={(e) => onChange(setTeamEffort(draft, t.id, { mode: "custom", budget_tokens: Number(e.target.value) || 0 }))}
                    style={advInp}
                  />
                </label>
              )}
              <label style={advLbl}>
                tools (comma-separated)
                <input
                  aria-label={`tools for ${t.id}`}
                  value={t.scope.tools.join(", ")}
                  onChange={(e) => onChange(setTeamTools(draft, t.id, e.target.value))}
                  style={advInp}
                />
              </label>
              <label style={advLbl}>
                reads (comma-separated)
                <input
                  aria-label={`reads for ${t.id}`}
                  value={t.scope.reads.join(", ")}
                  onChange={(e) => onChange(setTeamReads(draft, t.id, e.target.value))}
                  style={advInp}
                />
              </label>
              <label style={advLbl}>
                writes (comma-separated)
                <input
                  aria-label={`writes for ${t.id}`}
                  value={t.scope.writes.join(", ")}
                  onChange={(e) => onChange(setTeamWrites(draft, t.id, e.target.value))}
                  style={advInp}
                />
              </label>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

const advLbl: React.CSSProperties = { fontSize: "var(--ts-sm)", color: "var(--text-3)", display: "block" };
const advInp: React.CSSProperties = { width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", fontFamily: "inherit" };
