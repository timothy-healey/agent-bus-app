import type { CSSProperties } from "react";
import type { ReanchoredComment } from "../ipc/review";

export interface CommentRailProps {
  comments: ReanchoredComment[];
  onSelect: (commentId: string) => void;
  onDelete: (commentId: string) => void;
  activeId?: string;
}

/// A review comment's note is `<team>: <verdict>. <reason>`; split off the team
/// so it shows as the author.
export function splitReviewNote(note: string): { team: string; body: string } {
  const i = note.indexOf(": ");
  return i > 0 ? { team: note.slice(0, i), body: note.slice(i + 2) } : { team: "", body: note };
}

export function CommentRail({ comments, onSelect, onDelete, activeId }: CommentRailProps) {
  const inline = comments.filter((c) => c.kind === "inline");
  const reviews = comments.filter((c) => c.kind === "review");
  const addressed = inline.filter((c) => c.status === "addressed").length;

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

  const badge: CSSProperties = {
    marginLeft: 6,
    fontSize: 9.5,
    color: "var(--accent)",
    border: "1px solid var(--accent)",
    borderRadius: "var(--r-xs)",
    padding: "0 5px",
    textTransform: "lowercase",
  };
  const bodyStyle: CSSProperties = {
    color: "var(--text)",
    fontFamily: "ui-sans-serif, system-ui, sans-serif",
    fontSize: 12,
    marginLeft: 22,
    lineHeight: 1.45,
  };

  // A reviewer team's verdict and reason: not anchored to a span, so it is not
  // numbered and does not count toward the inline comments.
  const reviewEntries = reviews.map((c) => {
    const { team, body } = splitReviewNote(c.note);
    return (
      <div
        key={c.id}
        data-kind="review"
        style={{
          padding: "10px 14px",
          borderBottom: "1px solid var(--border)",
          background: c.id === activeId ? "var(--accent-2)" : "transparent",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", marginLeft: 22 }}>
          <span style={{ color: "var(--text-3)", fontSize: 10.5 }}>{team}</span>
          <span style={badge}>review</span>
        </div>
        <div style={{ ...bodyStyle, marginTop: 6 }}>{body}</div>
      </div>
    );
  });

  if (inline.length === 0 && reviews.length === 0) {
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
          none yet. select text to comment.
        </div>
      </div>
    );
  }

  return (
    <div style={rail}>
      <div style={head}>
        <span>comments</span>
        <span data-testid="rail-count" style={{ color: "var(--accent)" }}>
          {addressed > 0
            ? `${addressed} of ${inline.length} addressed`
            : `${inline.length} comment${inline.length === 1 ? "" : "s"}`}
        </span>
      </div>
      {reviewEntries}
      {inline.map((c, idx) => {
        const active = c.id === activeId;
        const isAddressed = c.status === "addressed";
        const entry: CSSProperties = {
          padding: "10px 14px",
          borderBottom: "1px solid var(--border)",
          cursor: "pointer",
          background: active
            ? "var(--accent-2)"
            : isAddressed
              ? "var(--bg-3, var(--bg-2))"
              : "transparent",
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
          <div key={c.id} data-kind="inline" data-status={c.status} style={entry} onClick={() => onSelect(c.id)}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <span>
                <span style={marker}>{idx + 1}</span>
                <span style={{ color: "var(--text-3)", fontSize: 10.5 }}>you</span>
                {isAddressed && (
                  <span
                    style={{
                      marginLeft: 6,
                      fontSize: 9.5,
                      color: "var(--accent)",
                      border: "1px solid var(--accent)",
                      borderRadius: "var(--r-xs)",
                      padding: "0 5px",
                      textTransform: "lowercase",
                    }}
                  >
                    addressed
                  </span>
                )}
              </span>
              <button
                type="button"
                className="abp-icon-delete"
                aria-label="delete comment"
                onClick={(e) => {
                  e.stopPropagation();
                  onDelete(c.id);
                }}
                style={{
                  background: "transparent",
                  border: "none",
                  color: "var(--text-3)",
                  cursor: "pointer",
                  fontFamily: "inherit",
                }}
              >
                ✕
              </button>
            </div>
            {c.anchor_text && <div style={quote}>"{c.anchor_text}"</div>}
            <div
              style={{
                ...noteStyle,
                textDecoration: isAddressed ? "line-through" : "none",
                opacity: isAddressed ? 0.7 : 1,
              }}
            >
              {c.note}
            </div>
          </div>
        );
      })}
    </div>
  );
}
