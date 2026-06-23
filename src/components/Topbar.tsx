import { type Project } from "../ipc/workspace";
import type { UsageSnapshot } from "../ipc/usage";
import { ThemeToggle } from "./ThemeToggle";
import { UsageMeter } from "./UsageMeter";
import { Button } from "./ui/Button";

export interface TopbarProps {
  activeProject: Project | null;
  onNewProject: () => void;
  usage: UsageSnapshot | null;
  brakeOn: boolean;
  brakeReason?: string;
  onToggleBrake: (next: boolean) => void;
}

export function Topbar({ activeProject, onNewProject, usage, brakeOn, brakeReason, onToggleBrake }: TopbarProps) {
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

      <div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: "var(--sp-4)" }}>
        <UsageMeter snapshot={usage} />

        <button
          onClick={() => onToggleBrake(!brakeOn)}
          aria-pressed={brakeOn}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 6,
            background: "transparent",
            border: "1px solid var(--border)",
            color: brakeOn ? "var(--danger)" : "var(--text-3)",
            padding: "4px 10px",
            fontFamily: "inherit",
            fontSize: "var(--ts-base)",
            borderRadius: "var(--r-sm)",
            cursor: "pointer",
          }}
        >
          <span
            style={{
              width: 8,
              height: 8,
              borderRadius: "50%",
              background: brakeOn ? "var(--danger)" : "var(--running)",
            }}
          />
          {brakeOn ? "brake on" : "brake off"}
          {brakeOn && brakeReason ? (
            <span style={{ color: "var(--text-3)", fontSize: 11 }}>(reason: {brakeReason})</span>
          ) : null}
        </button>

        <Button variant="primary" size="sm" onClick={onNewProject}>New project</Button>
        <ThemeToggle />
      </div>
    </header>
  );
}
