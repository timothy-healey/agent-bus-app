import type React from "react";
import type { EffortMode, Pipeline } from "../ipc/pipeline";

function effortLabel(e: EffortMode): string {
  return e.mode === "custom" ? `custom(${e.budget_tokens})` : e.mode;
}

const sectionTitle: React.CSSProperties = {
  fontSize: 11,
  textTransform: "uppercase",
  letterSpacing: "0.08em",
  color: "var(--text-3)",
  margin: "var(--sp-7) 0 var(--sp-3)",
};

const card: React.CSSProperties = {
  border: "1px solid var(--border)",
  borderRadius: "var(--r-md)",
  background: "var(--surface)",
  padding: "var(--sp-4) var(--sp-5)",
  marginBottom: "var(--sp-2)",
};

const meta: React.CSSProperties = { color: "var(--text-3)", fontSize: 11 };

const gateCard: React.CSSProperties = {
  ...card,
  background: "var(--accent-2)",
  borderColor: "var(--accent-bd)",
};

export interface PipelineViewProps {
  pipeline: Pipeline | null;
  /// When provided, an "Edit pipeline" affordance opens the in-app editor (A1).
  /// Omitted = pure read-only viewer (e.g. the wizard review step).
  onEdit?: () => void;
}

export function PipelineView({ pipeline, onEdit }: PipelineViewProps) {
  if (!pipeline) {
    return (
      <div style={{ padding: "var(--sp-8)", color: "var(--text-3)", textAlign: "center" }}>
        No pipeline loaded. Activate one from the project to view its graph.
      </div>
    );
  }

  return (
    <div style={{ padding: "var(--sp-7) var(--sp-8)", maxWidth: 760 }}>
      <div style={{ display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: "var(--sp-4)" }}>
        <div>
          <h1 style={{ fontSize: 16, color: "var(--text)", margin: 0 }}>{pipeline.name}</h1>
          {pipeline.description && (
            <p style={{ ...meta, marginTop: "var(--sp-1)" }}>{pipeline.description}</p>
          )}
          <div style={meta}>schema v{pipeline.schema_version}</div>
        </div>
        {onEdit && (
          <button
            onClick={onEdit}
            aria-label="edit pipeline"
            style={{ flexShrink: 0, background: "var(--surface-2)", border: "1px solid var(--border-2)", color: "var(--text)", padding: "4px 14px", borderRadius: "var(--r-sm)", fontSize: 12, cursor: "pointer" }}
          >
            Edit pipeline
          </button>
        )}
      </div>

      <div style={sectionTitle}>Teams ({pipeline.teams.length})</div>
      {pipeline.teams.map((t) => (
        <div key={t.id} style={card}>
          <div style={{ color: "var(--text)", fontWeight: 500 }}>
            {t.name} <span style={meta}>· {t.id}</span>
          </div>
          <div style={meta}>
            {t.runner?.kind ?? "—"} · {t.runner?.model ?? "—"} · effort{" "}
            {t.runner?.effort ? effortLabel(t.runner.effort) : "—"} · workers{" "}
            {t.workers.default}/{t.workers.max}
          </div>
          <div style={{ ...meta, marginTop: 4 }}>
            {t.outputs.on_approve && <span>approve → {t.outputs.on_approve}&nbsp;&nbsp;</span>}
            {t.outputs.on_revise && <span>revise → {t.outputs.on_revise}&nbsp;&nbsp;</span>}
            {t.outputs.on_reject && <span>reject → {t.outputs.on_reject}</span>}
          </div>
        </div>
      ))}

      <div style={sectionTitle}>Gates ({pipeline.gates.length})</div>
      {pipeline.gates.map((g) => (
        <div key={g.id} style={gateCard}>
          <div style={{ color: "var(--text)", fontWeight: 500 }}>{g.label}</div>
          <div style={meta}>
            {g.id} · downstream → {g.downstream}
          </div>
        </div>
      ))}

      <div style={sectionTitle}>Escalations ({pipeline.escalations.length})</div>
      {pipeline.escalations.map((e) => (
        <div key={e.id} style={card}>
          <div style={{ color: "var(--text)", fontWeight: 500 }}>{e.id}</div>
          <div style={meta}>triggers: {e.triggers.join(" · ") || "none"}</div>
        </div>
      ))}

      <div style={sectionTitle}>Forks ({pipeline.forks.length})</div>
      {pipeline.forks.map((f) => (
        <div key={f.id} style={card}>
          <div style={{ color: "var(--text)", fontWeight: 500 }}>{f.id}</div>
          <div style={meta}>lanes: {f.lanes.join(", ")}</div>
        </div>
      ))}

      <div style={sectionTitle}>Joins ({pipeline.joins.length})</div>
      {pipeline.joins.map((j) => (
        <div key={j.id} style={card}>
          <div style={{ color: "var(--text)", fontWeight: 500 }}>{j.id}</div>
          <div style={meta}>
            waits for: {j.waits_for.join(", ")} · downstream → {j.downstream}
          </div>
        </div>
      ))}
    </div>
  );
}
