import { useState, type CSSProperties } from "react";
import { Button } from "./ui/Button";

export interface ReviseComposePanelProps {
  commentCount: number;
  nextAttempts: number;
  target: string;
  onSend: (direction: string) => void;
  onCancel: () => void;
}

export function ReviseComposePanel({
  commentCount,
  nextAttempts,
  target,
  onSend,
  onCancel,
}: ReviseComposePanelProps) {
  const [direction, setDirection] = useState("");

  const root: CSSProperties = {
    borderTop: "1px solid var(--border)",
    background: "var(--surface-2)",
    padding: "14px 18px",
  };
  const h5: CSSProperties = {
    margin: "0 0 6px",
    fontSize: 11,
    color: "var(--text-3)",
    fontWeight: 400,
    textTransform: "lowercase",
    letterSpacing: "0.04em",
  };
  const summary: CSSProperties = {
    color: "var(--text-2)",
    fontFamily: "ui-sans-serif, system-ui, sans-serif",
    fontSize: 12,
    marginBottom: 12,
  };
  const textarea: CSSProperties = {
    width: "100%",
    background: "var(--bg-2)",
    border: "1px solid var(--border)",
    color: "var(--text)",
    fontFamily: "ui-sans-serif, system-ui, sans-serif",
    fontSize: 12.5,
    padding: "8px 10px",
    borderRadius: "var(--r-sm)",
    resize: "vertical",
    minHeight: 70,
    lineHeight: 1.5,
    boxSizing: "border-box",
  };
  const row: CSSProperties = {
    display: "flex",
    justifyContent: "flex-end",
    gap: 8,
    marginTop: 10,
  };

  return (
    <div style={root}>
      <h5 style={h5}>send back to {target}</h5>
      <div style={summary}>
        {commentCount} inline comments will be bundled with this version.{" "}
        {target}'s worker will read them as the basis for the next version.
        Attempts after: <strong style={{ color: "var(--accent)" }}>{nextAttempts}/3</strong>.
      </div>
      <h5 style={h5}>overall direction (optional)</h5>
      <textarea
        aria-label="overall revise direction"
        style={textarea}
        placeholder={`anything ${target} should know beyond the inline comments?`}
        value={direction}
        onChange={(e) => setDirection(e.target.value)}
      />
      <div style={row}>
        <Button variant="ghost" size="sm" onClick={onCancel}>
          cancel
        </Button>
        <Button variant="primary" size="sm" onClick={() => onSend(direction.trim())}>
          send back to {target}
        </Button>
      </div>
    </div>
  );
}
