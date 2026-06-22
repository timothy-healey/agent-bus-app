import type { CSSProperties } from "react";
import type { Comment } from "../ipc/review";

export interface CommentRailProps {
  comments: Comment[];
  onSelect: (commentId: string) => void;
  onDelete: (commentId: string) => void;
  activeId?: string;
}

export function CommentRail({ comments, onSelect, onDelete, activeId }: CommentRailProps) {
  const inline = comments.filter((c) => c.kind === "inline");

  const rail: CSSProperties = {
    background: "var(--bg-2)",
    borderLeft: "1px solid var(--border)",
    overflowY: "auto",
    height: "100%",
    fontFamily: "inherit",
  };
  const head: CSSProperties = {
    padding: "10px 14px",
    borderBottom: "1px solid var(--border)",
    fontSize: 10.5,
    color: "var(--text-3)",
    textTransform: "lowercase",
    letterSpacing: "0.03em",
    display: "flex",
    justifyContent: "space-between",
  };

  if (inline.length === 0) {
    return (
      <div style={rail}>
        <div style={head}>
          <span>comments</span>
          <span data-testid="rail-count" style={{ color: "var(--accent)" }}>0 comments</span>
        </div>
        <div
          style={{
            padding: 14,
            color: "var(--text-4)",
            fontSize: 11,
            textAlign: "center",
            fontStyle: "italic",
          }}
        >
          none yet — select text to comment
        </div>
      </div>
    );
  }

  return (
    <div style={rail}>
      <div style={head}>
        <span>comments</span>
        <span data-testid="rail-count" style={{ color: "var(--accent)" }}>
          {inline.length} comment{inline.length === 1 ? "" : "s"}
        </span>
      </div>
      {inline.map((c, idx) => {
        const active = c.id === activeId;
        const entry: CSSProperties = {
          padding: "10px 14px",
          borderBottom: "1px solid var(--border)",
          cursor: "pointer",
          background: active ? "var(--accent-2)" : "transparent",
        };
        const marker: CSSProperties = {
          display: "inline-block",
          background: "var(--accent)",
          color: "oklch(15% 0.04 55)",
          fontSize: 10,
          fontWeight: 700,
          width: 16,
          height: 16,
          lineHeight: "16px",
          textAlign: "center",
          borderRadius: "50%",
          marginRight: 6,
          verticalAlign: "middle",
        };
        const quote: CSSProperties = {
          color: "var(--text-3)",
          fontFamily: "ui-sans-serif, system-ui, sans-serif",
          fontSize: 11.5,
          fontStyle: "italic",
          margin: "6px 0 6px 22px",
          paddingLeft: 8,
          borderLeft: "1px solid var(--border)",
          lineHeight: 1.45,
        };
        const noteStyle: CSSProperties = {
          color: "var(--text)",
          fontFamily: "ui-sans-serif, system-ui, sans-serif",
          fontSize: 12,
          marginLeft: 22,
          lineHeight: 1.45,
        };
        return (
          <div key={c.id} style={entry} onClick={() => onSelect(c.id)}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <span>
                <span style={marker}>{idx + 1}</span>
                <span style={{ color: "var(--text-3)", fontSize: 10.5 }}>you</span>
              </span>
              <button
                type="button"
                aria-label="delete comment"
                onClick={(e) => {
                  e.stopPropagation();
                  onDelete(c.id);
                }}
                style={{
                  background: "transparent",
                  border: "none",
                  color: "var(--text-4)",
                  cursor: "pointer",
                  fontFamily: "inherit",
                }}
              >
                ✕
              </button>
            </div>
            {c.anchor_text && <div style={quote}>"{c.anchor_text}"</div>}
            <div style={noteStyle}>{c.note}</div>
          </div>
        );
      })}
    </div>
  );
}
