import { useState, type CSSProperties } from "react";
import { Button } from "./ui/Button";

export interface SelectionPopoverProps {
  quote: string;
  onAdd: (note: string) => void;
  onCancel: () => void;
}

export function SelectionPopover({ quote, onAdd, onCancel }: SelectionPopoverProps) {
  const [note, setNote] = useState("");

  const root: CSSProperties = {
    background: "var(--surface-3)",
    border: "1px solid var(--border-2)",
    borderRadius: "var(--r-sm)",
    padding: "10px 12px",
    boxShadow: "var(--shadow-popover)",
    width: 260,
    fontFamily: "inherit",
    fontSize: "var(--ts-sm)",
  };
  const label: CSSProperties = {
    fontSize: 10,
    color: "var(--text-3)",
    marginBottom: 6,
  };
  const quoted: CSSProperties = {
    color: "var(--text-3)",
    fontFamily: "ui-sans-serif, system-ui, sans-serif",
    fontSize: "var(--ts-sm)",
    padding: "4px 6px",
    borderLeft: "1px solid var(--border)",
    background: "var(--bg-2)",
    marginBottom: 8,
  };
  const textarea: CSSProperties = {
    width: "100%",
    background: "var(--bg-2)",
    border: "1px solid var(--border)",
    color: "var(--text)",
    fontFamily: "inherit",
    fontSize: 11.5,
    padding: "6px 8px",
    borderRadius: "var(--r-xs)",
    resize: "vertical",
    minHeight: 50,
    boxSizing: "border-box",
  };
  const actions: CSSProperties = {
    display: "flex",
    justifyContent: "flex-end",
    gap: 6,
    marginTop: 8,
  };

  return (
    <div style={root}>
      <div style={label}>comment on selection</div>
      <div style={quoted}>"{quote}"</div>
      <textarea
        autoFocus
        aria-label="comment note"
        placeholder="your note…"
        style={textarea}
        value={note}
        onChange={(e) => setNote(e.target.value)}
      />
      <div style={actions}>
        <Button variant="ghost" size="sm" onClick={onCancel}>
          cancel
        </Button>
        <Button
          variant="primary"
          size="sm"
          onClick={() => {
            if (note.trim()) onAdd(note.trim());
          }}
        >
          add comment
        </Button>
      </div>
    </div>
  );
}
