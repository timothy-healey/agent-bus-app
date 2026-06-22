import type { CSSProperties } from "react";
import type { TaskState } from "../../ipc/runtime";

const labels: Record<TaskState, string> = {
  queued: "queued",
  running: "running",
  gated: "needs you",
  revising: "revise",
  needs_human: "needs human",
  done: "done",
  braked: "braked",
};

export function stateLabel(state: TaskState): string {
  return labels[state];
}

const dotColors: Record<TaskState, string> = {
  queued: "var(--text-3)",
  running: "var(--running)",
  gated: "var(--accent)",
  revising: "var(--revise)",
  needs_human: "var(--danger)",
  done: "var(--text-3)",
  braked: "var(--text-4)",
};

export interface StatePillProps {
  state: TaskState;
}

export function StatePill({ state }: StatePillProps) {
  const wrap: CSSProperties = {
    display: "inline-flex",
    alignItems: "center",
    gap: 6,
    fontSize: 10,
    color: "var(--text-2)",
    fontFamily: "inherit",
  };
  const dot: CSSProperties = {
    width: 6,
    height: 6,
    borderRadius: "50%",
    background: dotColors[state],
    flexShrink: 0,
  };
  return (
    <span style={wrap}>
      <span data-dot style={dot} />
      {labels[state]}
    </span>
  );
}
