import type { CSSProperties } from "react";
import { parseMarkdown } from "../lib/markdown";
import { renderBlock } from "../lib/renderBlock";
import { lineDiff } from "../lib/lineDiff";

export interface ComparePane {
  label: string;
  markdown: string;
}

export interface CompareViewProps {
  left: ComparePane;
  right: ComparePane;
}

/// Read-only two-column side-by-side render of two artifact versions (B2).
/// Reuses the shared markdown block renderer; a per-side change badge is
/// derived from the pure line diff. No comment/selection affordance (that is
/// ArtifactView-only) — this is a pure read projection over read_artifact.
export function CompareView({ left, right }: CompareViewProps) {
  const diff = lineDiff(left.markdown, right.markdown);

  const wrap: CSSProperties = {
    flex: 1,
    display: "flex",
    minHeight: 0,
    overflow: "hidden",
  };
  const col: CSSProperties = {
    flex: 1,
    minWidth: 0,
    display: "flex",
    flexDirection: "column",
  };
  const divider: CSSProperties = {
    width: 1,
    background: "var(--border)",
    flexShrink: 0,
  };
  const colHead: CSSProperties = {
    display: "flex",
    alignItems: "center",
    gap: 8,
    padding: "8px 14px",
    borderBottom: "1px solid var(--border)",
    background: "var(--bg-2)",
  };
  const body: CSSProperties = {
    flex: 1,
    overflowY: "auto",
    padding: "14px 18px",
    fontFamily: "var(--font-reading)",
    fontSize: 13,
    lineHeight: 1.6,
    color: "var(--text-2)",
  };

  function badgeStyle(changed: number): CSSProperties {
    return {
      marginLeft: "auto",
      fontSize: "var(--ts-xs)",
      fontFamily: "var(--font-mono)",
      color: changed > 0 ? "var(--accent)" : "var(--text-4)",
    };
  }

  function pane(p: ComparePane, side: "left" | "right") {
    // Per-side badge: the left pane "owns" removals, the right pane additions.
    const added = side === "right" ? diff.added : 0;
    const removed = side === "left" ? diff.removed : 0;
    const parts: string[] = [];
    if (removed > 0) parts.push(`-${removed}`);
    if (added > 0) parts.push(`+${added}`);
    const badgeText = parts.length > 0 ? parts.join(" ") : "no changes";
    return (
      <div style={col}>
        <div style={colHead}>
          <span style={{ color: "var(--text-2)", fontSize: 11.5 }}>{p.label}</span>
          <span data-testid={`compare-badge-${side}`} style={badgeStyle(added + removed)}>
            {badgeText}
          </span>
        </div>
        {p.markdown.trim() ? (
          <div style={body}>{parseMarkdown(p.markdown).map(renderBlock)}</div>
        ) : (
          <div style={{ ...body, color: "var(--text-4)", fontStyle: "italic", fontSize: 12 }}>
            no artifact at this version.
          </div>
        )}
      </div>
    );
  }

  return (
    <div style={wrap} data-testid="compare-view">
      {pane(left, "left")}
      <div style={divider} />
      {pane(right, "right")}
    </div>
  );
}
