import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { useModalA11y } from "../../hooks/useModalA11y";

export interface DrawerProps {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
  /// Accessible name for the dialog. Defaults to a generic label.
  label?: string;
}

export function Drawer({ open, onClose, children, label = "detail" }: DrawerProps) {
  // Slide the panel in from the right on mount (DESIGN.md §Motion). Reduced
  // motion is honoured by the .abp-drawer-enter-active class (global.css guard).
  const [entered, setEntered] = useState(false);
  // Focus trap + Escape-to-close + restore-focus (audit A1).
  const panelRef = useModalA11y<HTMLElement>(open, onClose);
  useEffect(() => {
    if (!open) {
      setEntered(false);
      return;
    }
    const id = requestAnimationFrame(() => setEntered(true));
    return () => cancelAnimationFrame(id);
  }, [open]);

  if (!open) return null;

  const backdrop: CSSProperties = {
    position: "fixed",
    inset: 0,
    background: "var(--scrim)",
    zIndex: 40,
  };
  const panel: CSSProperties = {
    position: "fixed",
    top: 0,
    right: 0,
    bottom: 0,
    width: "clamp(600px, 60vw, 960px)",
    background: "var(--surface)",
    borderLeft: "1px solid var(--border)",
    boxShadow: "var(--shadow-card)",
    zIndex: 41,
    display: "flex",
    flexDirection: "column",
    fontFamily: "inherit",
    transform: entered ? "translateX(0)" : "translateX(100%)",
    transition: "transform var(--dur-slow) var(--ease-out)",
  };
  const closeBtn: CSSProperties = {
    position: "absolute",
    top: 10,
    right: 12,
    background: "transparent",
    border: "none",
    color: "var(--text-3)",
    fontSize: 16,
    cursor: "pointer",
    fontFamily: "inherit",
    zIndex: 1,
  };

  return (
    <>
      <div data-testid="drawer-backdrop" style={backdrop} onClick={onClose} />
      <aside
        ref={panelRef}
        className="abp-drawer-enter-active"
        style={panel}
        role="dialog"
        aria-modal="true"
        aria-label={label}
      >
        <button
          type="button"
          aria-label="close"
          style={closeBtn}
          onClick={onClose}
        >
          ✕
        </button>
        {children}
      </aside>
    </>
  );
}
