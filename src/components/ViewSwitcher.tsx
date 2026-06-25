import { useRef } from "react";
import { rovingTabKey } from "../lib/roving";

export type View = "board" | "list" | "pipeline" | "settings";

export interface ViewSwitcherProps {
  active: View;
  onChange: (v: View) => void;
}

const tabs: { id: View; label: string; right?: boolean }[] = [
  { id: "board", label: "board" },
  { id: "list", label: "list" },
  { id: "pipeline", label: "pipeline" },
  { id: "settings", label: "settings", right: true },
];

export function ViewSwitcher({ active, onChange }: ViewSwitcherProps) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const activeIndex = tabs.findIndex((t) => t.id === active);

  function onKeyDown(e: React.KeyboardEvent<HTMLButtonElement>) {
    const next = rovingTabKey(e.key, activeIndex, tabs.length);
    if (next === null) return;
    e.preventDefault();
    onChange(tabs[next].id);
    refs.current[next]?.focus();
  }

  return (
    <div
      role="tablist"
      style={{
        display: "flex",
        gap: "var(--sp-1)",
        padding: "var(--sp-2) var(--sp-8)",
        borderBottom: "1px solid var(--border)",
        background: "var(--bg-2)",
      }}
    >
      {tabs.map((t, i) => {
        const selected = t.id === active;
        return (
          <button
            key={t.id}
            ref={(el) => {
              refs.current[i] = el;
            }}
            role="tab"
            aria-selected={selected}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(t.id)}
            onKeyDown={onKeyDown}
            style={{
              marginLeft: t.right ? "auto" : undefined,
              background: selected ? "var(--surface-2)" : "transparent",
              border: "1px solid",
              borderColor: selected ? "var(--border-2)" : "transparent",
              color: selected ? "var(--text)" : "var(--text-3)",
              padding: "3px 12px",
              fontFamily: "inherit",
              fontSize: 12,
              borderRadius: "var(--r-sm)",
              cursor: "pointer",
            }}
          >
            {t.label}
          </button>
        );
      })}
    </div>
  );
}
