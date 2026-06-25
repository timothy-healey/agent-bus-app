import { useEffect, useRef, type CSSProperties } from "react";

/// One action in the node context menu (G9). Structured as a list so future
/// actions (duplicate, disconnect, …) slot in as more entries — the menu is not
/// hard-coded to Delete.
export interface NodeMenuAction {
  id: string;
  label: string;
  onSelect: () => void;
  /// Destructive actions render in the danger treatment (DESIGN.md §States).
  destructive?: boolean;
}

export interface NodeContextMenuProps {
  /// Viewport coordinates the menu opens at (the right-click position).
  x: number;
  y: number;
  /// The node the menu acts on, used only for the accessible name.
  nodeId: string;
  actions: NodeMenuAction[];
  onClose: () => void;
}

/// A small right-click context menu for a canvas node (G9). A `menu`/`menuitem`
/// roled list, keyboard-navigable (↑/↓ + Enter), Escape-to-close, and
/// click-outside-to-close. Extensible via the `actions` list.
export function NodeContextMenu({ x, y, nodeId, actions, onClose }: NodeContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);

  // Focus the first item on open so the menu is immediately keyboard-driveable.
  useEffect(() => {
    const first = ref.current?.querySelector<HTMLButtonElement>('[role="menuitem"]');
    first?.focus();
  }, []);

  // Click-outside + Escape close (Escape handled here so it doesn't bubble to the
  // full-page view's quick-exit).
  useEffect(() => {
    function onDocPointer(e: MouseEvent) {
      if (!ref.current?.contains(e.target as Node)) onClose();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    }
    document.addEventListener("mousedown", onDocPointer, true);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("mousedown", onDocPointer, true);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [onClose]);

  // ↑/↓ roving focus between the menu items.
  function onMenuKeyDown(e: React.KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);
    if (items.length === 0) return;
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    const delta = e.key === "ArrowDown" ? 1 : -1;
    const next = (i + delta + items.length) % items.length;
    items[next].focus();
  }

  return (
    <div
      ref={ref}
      role="menu"
      aria-label={`actions for ${nodeId}`}
      style={{ ...menu, left: x, top: y }}
      onKeyDown={onMenuKeyDown}
    >
      {actions.map((a) => (
        <button
          key={a.id}
          type="button"
          role="menuitem"
          className="abp-context-item"
          style={{ ...item, color: a.destructive ? "var(--danger)" : "var(--text-2)" }}
          onClick={() => {
            a.onSelect();
            onClose();
          }}
        >
          {a.label}
        </button>
      ))}
    </div>
  );
}

const menu: CSSProperties = {
  position: "fixed",
  zIndex: 60,
  minWidth: 160,
  padding: "var(--sp-1)",
  background: "var(--surface)",
  border: "1px solid var(--border)",
  borderRadius: "var(--r-sm)",
  boxShadow: "var(--shadow-popover)",
  display: "flex",
  flexDirection: "column",
  gap: 2,
};
const item: CSSProperties = {
  display: "block",
  width: "100%",
  textAlign: "left",
  fontFamily: "inherit",
  fontSize: "var(--ts-base)",
  padding: "var(--sp-2) var(--sp-3)",
  background: "transparent",
  border: "none",
  borderRadius: "var(--r-sm)",
  cursor: "pointer",
};
