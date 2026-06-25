import { useId, useState, type CSSProperties, type KeyboardEvent } from "react";

/// InfoTip (G8) — an accessible informational `?` affordance with concise help.
/// A button-triggered popover that toggles on click and Enter/Space, dismisses
/// on Escape, and is described to the field via `aria-describedby` when the
/// caller wires the returned id. Tokenized; keyboard-reachable (inherits the
/// global focus-visible ring). Informational only — no actions inside.
export interface InfoTipProps {
  /// The help text shown in the popover.
  text: string;
  /// Accessible label for the trigger, e.g. "help for Reads". Defaults generic.
  label?: string;
}

export function InfoTip({ text, label = "more info" }: InfoTipProps) {
  const [open, setOpen] = useState(false);
  const tipId = useId();

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") setOpen(false);
  }

  return (
    <span style={wrap} onKeyDown={onKey}>
      <button
        type="button"
        aria-label={label}
        aria-expanded={open}
        aria-describedby={open ? tipId : undefined}
        onClick={() => setOpen((o) => !o)}
        style={trigger}
      >
        ?
      </button>
      {open && (
        <span role="tooltip" id={tipId} style={pop}>
          {text}
        </span>
      )}
    </span>
  );
}

const wrap: CSSProperties = { position: "relative", display: "inline-flex" };
const trigger: CSSProperties = {
  width: 15,
  height: 15,
  lineHeight: "13px",
  borderRadius: "50%",
  border: "1px solid var(--border-2)",
  background: "var(--surface-2)",
  color: "var(--text-3)",
  fontSize: 10,
  cursor: "pointer",
  padding: 0,
  fontFamily: "inherit",
  textAlign: "center",
};
const pop: CSSProperties = {
  position: "absolute",
  top: "calc(100% + 4px)",
  left: 0,
  zIndex: 50,
  width: "max-content",
  maxWidth: 260,
  background: "var(--surface-3)",
  border: "1px solid var(--border-2)",
  borderRadius: "var(--r-sm)",
  boxShadow: "var(--shadow-popover)",
  color: "var(--text-2)",
  fontSize: "var(--ts-sm)",
  lineHeight: 1.4,
  padding: "var(--sp-2) var(--sp-3)",
  fontWeight: 400,
};
