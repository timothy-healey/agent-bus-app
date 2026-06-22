import { type Project } from "../ipc/workspace";
import { ThemeToggle } from "./ThemeToggle";

export interface TopbarProps {
  activeProject: Project | null;
  onNewProject: () => void;
}

export function Topbar({ activeProject, onNewProject }: TopbarProps) {
  return (
    <header
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-5)",
        padding: "var(--sp-3) var(--sp-8)",
        borderBottom: "1px solid var(--border)",
        background: "var(--surface)",
      }}
    >
      <div style={{ fontWeight: 600, fontSize: 14, color: "var(--text)" }}>
        <span style={{ color: "var(--accent)" }}>●</span>&nbsp;agent bus
      </div>

      <div
        className="mono"
        style={{
          color: "var(--text-2)",
          fontSize: 12,
          padding: "2px 10px",
          border: "1px solid var(--border)",
          borderRadius: "var(--r-pill)",
          background: "var(--bg)",
        }}
      >
        {activeProject ? activeProject.name : "No project"}
      </div>

      <div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: "var(--sp-2)" }}>
        <button
          onClick={onNewProject}
          style={{
            background: "var(--accent)",
            border: "1px solid var(--accent)",
            color: "oklch(15% 0.04 55)",
            padding: "4px 10px",
            fontFamily: "inherit",
            fontSize: 11,
            borderRadius: "var(--r-sm)",
            cursor: "pointer",
            fontWeight: 500,
          }}
        >
          New project
        </button>
        <ThemeToggle />
      </div>
    </header>
  );
}
