import type { CSSProperties } from "react";
import type { Task } from "../ipc/runtime";
import { buildLineage } from "../lib/lineage";

export interface LineageTabProps {
  task: Task;
  /// Open an artifact path in the artifact pane (D5: single-pane, no compare yet).
  onOpenArtifact: (path: string) => void;
}

export function LineageTab({ task, onOpenArtifact }: LineageTabProps) {
  const chain = buildLineage(task);
  const hasArtifacts = chain.some((e) => e.path);

  const row: CSSProperties = {
    display: "flex", alignItems: "baseline", gap: 10, padding: "8px 0",
    borderBottom: "1px solid var(--border)",
  };
  const dot: CSSProperties = { color: "var(--text-4)", fontSize: 10 };

  return (
    <div style={{ flex: 1, padding: "14px 18px", overflowY: "auto" }}>
      {chain.map((e, i) => (
        <div key={`${e.kind}-${i}`} style={row}>
          <span style={dot}>{i === 0 ? "●" : "↳"}</span>
          {e.path ? (
            <button
              onClick={() => onOpenArtifact(e.path!)}
              style={{
                background: "none", border: "none", padding: 0, cursor: "pointer",
                color: "var(--accent)", fontSize: 12, fontFamily: "inherit", textAlign: "left",
              }}
            >
              {e.label}
            </button>
          ) : (
            <span style={{ color: "var(--text-2)", fontSize: 12 }}>{e.label}</span>
          )}
          {e.kind !== "topic" && (
            <span style={{ marginLeft: "auto", color: "var(--text-3)", fontSize: 10 }}>{e.kind}</span>
          )}
        </div>
      ))}
      {!hasArtifacts && (
        <div style={{ color: "var(--text-3)", fontSize: 11, marginTop: 10 }}>
          no upstream artifacts yet. this task has not produced one.
        </div>
      )}
    </div>
  );
}
