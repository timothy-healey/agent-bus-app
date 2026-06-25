import type { CSSProperties } from "react";
import type { Task } from "../../ipc/runtime";
import { StatePill } from "./StatePill";
import { costColorVar, costBand, formatTokens } from "../../lib/cost";

export interface CardProps {
  task: Task;
  tokens: number;
  onClick?: (taskId: string) => void;
  /// Optional title override (④e: the board labels work-item cards by their
  /// `item_key`). Falls back to the task's topic when absent.
  label?: string;
}

/// Per-state border + background treatment (DESIGN.md §Card). Status comes from
/// the StatePill dot; the border shifts only for needs-you / revise / braked.
function shell(state: Task["state"]): CSSProperties {
  switch (state) {
    case "gated":
      return {
        borderColor: "var(--accent-bd)",
        background: "var(--surface)",
        boxShadow: "var(--shadow-needs-you)",
      };
    case "revising":
      return { borderColor: "var(--revise)", background: "var(--revise-2)" };
    case "needs_human":
      return { borderColor: "var(--danger-2)", background: "var(--surface)" };
    case "braked":
      return { borderColor: "var(--border)", background: "var(--surface)", opacity: 0.6 };
    default:
      return { borderColor: "var(--border)", background: "var(--surface)" };
  }
}

export function Card({ task, tokens, onClick, label }: CardProps) {
  const root: CSSProperties = {
    border: "1px solid",
    borderRadius: "var(--r-md)",
    padding: "10px 12px",
    marginBottom: 6,
    cursor: onClick ? "pointer" : "default",
    fontFamily: "inherit",
    ...shell(task.state),
  };
  const topRow: CSSProperties = {
    display: "flex",
    justifyContent: "space-between",
    alignItems: "center",
    gap: 8,
  };
  const title: CSSProperties = {
    color: "var(--text)",
    fontSize: 13,
    fontWeight: 450,
    lineHeight: 1.35,
    marginTop: 8,
  };
  const meta: CSSProperties = {
    display: "flex",
    justifyContent: "space-between",
    alignItems: "center",
    gap: 8,
    marginTop: 12,
    fontSize: 10,
    color: "var(--text-3)",
    fontVariantNumeric: "tabular-nums",
  };
  const cost: CSSProperties = { color: costColorVar[costBand(tokens)] };

  return (
    <div className="abp-card" style={root} onClick={() => onClick?.(task.id)}>
      <div style={topRow}>
        <code style={{ color: "var(--text-3)", fontSize: 11, fontFamily: "var(--font-mono)" }}>
          {task.state === "revising" ? "↩ " : ""}
          {task.item_key ?? ""}
        </code>
        <StatePill state={task.state} />
      </div>
      <div style={title}>{label ?? task.topic}</div>
      <div style={meta}>
        <span style={cost}>{formatTokens(tokens)} tok</span>
        <span>a{task.attempts}</span>
      </div>
    </div>
  );
}
