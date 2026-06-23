import type { CSSProperties } from "react";
import type { Pipeline } from "../ipc/pipeline";
import type { Task } from "../ipc/runtime";
import { buildLanes, type Lane } from "../lib/lanes";
import { Card } from "./ui/Card";

export interface BoardViewProps {
  pipeline: Pipeline | null;
  tasks: Task[];
  onOpenCard: (taskId: string) => void;
  /// token cost per task id (from Usage Telemetry in Plan 5; defaults to 0).
  tokensByTask?: Record<string, number>;
}

function laneHeaderStyle(lane: Lane): CSSProperties {
  // Only the gate lane earns the one accent (Decision 5): it is the truly
  // actionable "needs you" lane. Other non-team lanes differentiate by weight.
  const isGate = lane.kind === "gate";
  const emphasized = lane.kind !== "team";
  return {
    fontSize: "var(--ts-xs)",
    color: isGate ? "var(--accent)" : emphasized ? "var(--text-2)" : "var(--text-3)",
    fontWeight: emphasized ? 500 : 400,
    textTransform: "lowercase",
    letterSpacing: "0.04em",
    marginBottom: 8,
    padding: isGate ? "4px 8px" : "0 2px",
    background: isGate ? "var(--accent-2)" : "transparent",
    borderRadius: "var(--r-sm)",
  };
}

export function BoardView({
  pipeline,
  tasks,
  onOpenCard,
  tokensByTask = {},
}: BoardViewProps) {
  if (!pipeline) {
    return (
      <div style={{ padding: "var(--sp-10)", color: "var(--text-3)", fontSize: 12 }}>
        no active pipeline. create or activate one to see the board.
      </div>
    );
  }

  const lanes = buildLanes(pipeline, tasks);

  const board: CSSProperties = {
    display: "flex",
    gap: 12,
    padding: "var(--sp-5) var(--sp-8)",
    overflowX: "auto",
    alignItems: "flex-start",
  };
  const lane: CSSProperties = {
    minWidth: 200,
    maxWidth: 240,
    flexShrink: 0,
  };

  return (
    <div style={board}>
      {lanes.map((l) => (
        <div key={l.id} style={lane}>
          <div style={laneHeaderStyle(l)}>{l.label}</div>
          {l.tasks.map((t) => (
            <Card
              key={t.id}
              task={t}
              tokens={tokensByTask[t.id] ?? 0}
              onClick={onOpenCard}
            />
          ))}
        </div>
      ))}
    </div>
  );
}
