import type { CSSProperties, ReactNode } from "react";
import { useEscapeClose } from "../hooks/useEscapeClose";
import { Button } from "../components/ui/Button";

/// One entry in the left-nav step tree.
export interface NavStep {
  id: string;
  label: string;
  /// Jump straight to this step (G14 — free navigation; B2 refines gating).
  onSelect: () => void;
  /// Greyed + non-interactive when true (reserved for B2's validity gating).
  disabled?: boolean;
}

export interface AuthoringLayoutProps {
  /// Heading for the whole authoring view.
  title: string;
  /// The clickable nav tree (G14): Basics / Canvas / Review (or edit-mode's set).
  steps: NavStep[];
  /// The id of the currently shown step (marked current in the tree).
  currentStepId: string;
  /// The project switcher slot (G13/G14) — rendered beneath the nav tree.
  switcher?: ReactNode;
  /// Footer continue/back buttons, rendered beneath the switcher.
  footer?: ReactNode;
  /// The main pane: the current step's content (Basics form / Canvas / Review).
  children: ReactNode;
  /// Close/leave the authoring view (Cancel + Escape route here).
  onClose: () => void;
}

/// Shared full-page authoring layout (G10/G14). A left nav column (nav tree +
/// project switcher + back/continue) beside a main pane that hosts the current
/// step. BOTH the new-project flow and pipeline edit-mode render this instead of
/// a centered modal — so the canvas gets the full page (the operator's ask) and
/// the host stays stable for B2/B4 to layer on.
///
/// Focus/Escape behaviour suits a full-page view, NOT a modal: no focus trap, no
/// auto-focus steal, no `aria-modal`. Escape is a quick exit only (useEscapeClose).
export function AuthoringLayout({
  title,
  steps,
  currentStepId,
  switcher,
  footer,
  children,
  onClose,
}: AuthoringLayoutProps) {
  useEscapeClose(true, onClose);

  return (
    <div style={page} role="region" aria-label={title}>
      <aside style={nav} aria-label="Authoring navigation">
        <div style={navHead}>
          <h2 style={titleStyle}>{title}</h2>
          <Button variant="ghost" size="sm" onClick={onClose} aria-label="close authoring view">close</Button>
        </div>

        <nav aria-label="Steps">
          <ol style={tree}>
            {steps.map((s) => {
              const current = s.id === currentStepId;
              return (
                <li key={s.id}>
                  <button
                    type="button"
                    className="abp-navstep"
                    aria-current={current ? "step" : undefined}
                    disabled={s.disabled}
                    onClick={s.disabled ? undefined : s.onSelect}
                    style={{
                      ...navStep,
                      color: current ? "var(--text)" : s.disabled ? "var(--text-4)" : "var(--text-2)",
                      background: current ? "var(--surface-2)" : "transparent",
                      borderLeft: current ? "2px solid var(--accent)" : "2px solid transparent",
                      cursor: s.disabled ? "not-allowed" : "pointer",
                    }}
                  >
                    {s.label}
                  </button>
                </li>
              );
            })}
          </ol>
        </nav>

        {switcher && <div style={switcherSlot}>{switcher}</div>}

        {footer && <div style={footerSlot}>{footer}</div>}
      </aside>

      <main style={main}>{children}</main>
    </div>
  );
}

const page: CSSProperties = {
  position: "absolute",
  inset: 0,
  display: "flex",
  background: "var(--bg)",
  zIndex: 50,
};
const nav: CSSProperties = {
  width: 240,
  flexShrink: 0,
  display: "flex",
  flexDirection: "column",
  gap: "var(--sp-5)",
  padding: "var(--sp-5)",
  borderRight: "1px solid var(--border)",
  background: "var(--surface)",
  overflowY: "auto",
};
const navHead: CSSProperties = { display: "flex", alignItems: "center", justifyContent: "space-between", gap: "var(--sp-2)" };
const titleStyle: CSSProperties = { fontSize: "var(--ts-lg)", color: "var(--text)", margin: 0 };
const tree: CSSProperties = { listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: "var(--sp-1)" };
const navStep: CSSProperties = {
  display: "block",
  width: "100%",
  textAlign: "left",
  fontFamily: "inherit",
  fontSize: "var(--ts-base)",
  padding: "var(--sp-2) var(--sp-3)",
  borderTop: "none",
  borderRight: "none",
  borderBottom: "none",
  borderRadius: "var(--r-sm)",
  textTransform: "capitalize",
};
const switcherSlot: CSSProperties = { borderTop: "1px solid var(--border)", paddingTop: "var(--sp-4)" };
const footerSlot: CSSProperties = { marginTop: "auto", display: "flex", flexDirection: "column", gap: "var(--sp-2)" };
const main: CSSProperties = { flex: 1, minWidth: 0, display: "flex", flexDirection: "column", overflow: "hidden", padding: "var(--sp-7)" };
