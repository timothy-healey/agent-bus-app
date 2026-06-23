import { useState, type CSSProperties } from "react";
import type { Task } from "../ipc/runtime";
import { buildLineage } from "../lib/lineage";

export interface LineageTabProps {
  task: Task;
  /// Open a single artifact path in the artifact pane (D5: single-pane).
  onOpenArtifact: (path: string) => void;
  /// Enter side-by-side compare for two chosen artifact paths (B2).
  onCompare?: (pathA: string, pathB: string) => void;
}

export function LineageTab({ task, onOpenArtifact, onCompare }: LineageTabProps) {
  const chain = buildLineage(task);
  const hasArtifacts = chain.some((e) => e.path);
  const artifactCount = chain.filter((e) => e.path).length;
  const canCompare = onCompare != null && artifactCount >= 2;

  const [comparing, setComparing] = useState(false);
  // Up to two selected paths, FIFO (DD5): a third pick drops the oldest.
  const [picked, setPicked] = useState<string[]>([]);

  function toggle(path: string) {
    setPicked((prev) => {
      if (prev.includes(path)) return prev.filter((p) => p !== path);
      const next = [...prev, path].slice(-2);
      if (next.length === 2) onCompare?.(next[0], next[1]);
      return next;
    });
  }

  function exitCompare() {
    setComparing(false);
    setPicked([]);
  }

  const row: CSSProperties = {
    display: "flex", alignItems: "baseline", gap: 10, padding: "8px 0",
    borderBottom: "1px solid var(--border)",
  };
  const dot: CSSProperties = { color: "var(--text-4)", fontSize: 10 };

  return (
    <div style={{ flex: 1, padding: "14px 18px", overflowY: "auto" }}>
      {canCompare && (
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 8 }}>
          <button
            onClick={() => (comparing ? exitCompare() : setComparing(true))}
            style={{
              background: comparing ? "var(--accent)" : "none",
              border: "1px solid var(--border)",
              borderRadius: "var(--r-sm)",
              padding: "3px 10px", cursor: "pointer",
              color: comparing ? "var(--bg)" : "var(--text-2)",
              fontSize: 11, fontFamily: "inherit",
            }}
          >
            {comparing ? "exit compare" : "compare versions"}
          </button>
          {comparing && (
            <span style={{ color: "var(--text-3)", fontSize: 10 }}>
              pick two artifacts ({picked.length}/2)
            </span>
          )}
        </div>
      )}

      {chain.map((e, i) => (
        <div key={`${e.kind}-${i}`} style={row}>
          <span style={dot}>{i === 0 ? "●" : "↳"}</span>
          {comparing && e.path ? (
            <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
              <input
                type="checkbox"
                aria-label={e.label}
                checked={picked.includes(e.path)}
                onChange={() => toggle(e.path!)}
              />
              <span style={{ color: "var(--text-2)", fontSize: 12 }}>{e.label}</span>
            </label>
          ) : e.path ? (
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
