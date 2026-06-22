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
      {tabs.map((t) => {
        const selected = t.id === active;
        return (
          <button
            key={t.id}
            role="tab"
            aria-selected={selected}
            onClick={() => onChange(t.id)}
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
