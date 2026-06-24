import type { CSSProperties } from "react";
import type { Pipeline } from "../ipc/pipeline";
import type { Task, StoreOccupancy } from "../ipc/runtime";
import { buildLanes, type Lane } from "../lib/lanes";
import { Card } from "./ui/Card";

export interface BoardViewProps {
  pipeline: Pipeline | null;
  tasks: Task[];
  onOpenCard: (taskId: string) => void;
  /// token cost per task id (from Usage Telemetry in Plan 5; defaults to 0).
  tokensByTask?: Record<string, number>;
  /// Per-stage store occupancy + capacity for the selected run (④e). Team lane
  /// headers render "n/cap" from this. Empty when no run is scoped.
  occupancy?: StoreOccupancy[];
  /// Whether a run is currently scoped. When false (no run selected/started) the
  /// board shows a run-scoped empty hint rather than bare lanes.
  hasRun?: boolean;
}

/// The board labels a work-item card by its `item_key` (the work's stable lineage
/// identity — ④e); legacy/topic tasks fall back to their topic, then their id so
/// a card is never blank.
export function cardLabel(task: Task): string {
  return task.item_key?.trim() || task.topic?.trim() || task.id;
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
  occupancy = [],
  hasRun = false,
}: BoardViewProps) {
  if (!pipeline) {
    return (
      <div style={{ padding: "var(--sp-10)", color: "var(--text-3)", fontSize: "var(--ts-base)" }}>
        no active pipeline. create or activate one to see the board.
      </div>
    );
  }

  // No run scoped yet: a single teaching empty state rather than a wall of empty
  // lanes that imply data. Mirrors the "no active pipeline" hint's calm treatment.
  if (!hasRun) {
    return (
      <div style={{ padding: "var(--sp-10)", color: "var(--text-3)", fontSize: "var(--ts-base)" }}>
        no run scoped. start a run to populate the board.
      </div>
    );
  }

  const lanes = buildLanes(pipeline, tasks);
  // Per-team lookups for the lane indicators: store occupancy (keyed by stage)
  // and the team's worker ceiling (Workers.max).
  const occByStage = new Map(occupancy.map((o) => [o.stage, o]));
  const maxByTeam = new Map(pipeline.teams.map((t) => [t.id, t.workers?.max ?? 0]));

  const board: CSSProperties = {
    display: "flex",
    gap: 12,
    padding: "var(--sp-5) var(--sp-8)",
    overflowX: "auto",
    alignItems: "flex-start",
  };
  const laneStyle: CSSProperties = {
    minWidth: 200,
    maxWidth: 240,
    flexShrink: 0,
  };
  const indicators: CSSProperties = {
    display: "flex",
    gap: "var(--sp-3)",
    marginBottom: "var(--sp-2)",
    fontSize: "var(--ts-xs)",
    color: "var(--text-3)",
    fontVariantNumeric: "tabular-nums",
  };
  const metric: CSSProperties = { display: "inline-flex", alignItems: "center", gap: "var(--sp-1)" };

  return (
    <div style={board}>
      {lanes.map((l) => {
        const occ = l.kind === "team" ? occByStage.get(l.id) : undefined;
        // Pool busy = running work-items currently at this team's stage.
        const busy = l.kind === "team" ? l.tasks.filter((t) => t.state === "running").length : 0;
        const max = l.kind === "team" ? maxByTeam.get(l.id) ?? 0 : 0;
        return (
          <div key={l.id} style={laneStyle}>
            <div style={laneHeaderStyle(l)}>{l.label}</div>
            {l.kind === "team" && (
              <div style={indicators}>
                <span
                  style={metric}
                  title="store occupancy / capacity"
                  role="progressbar"
                  aria-label={`${l.label} store`}
                  aria-valuemin={0}
                  aria-valuemax={occ?.capacity ?? 0}
                  aria-valuenow={occ?.occupancy ?? 0}
                  aria-valuetext={occ ? `${occ.occupancy} of ${occ.capacity}` : "empty"}
                >
                  store {occ ? `${occ.occupancy}/${occ.capacity}` : "—"}
                </span>
                <span style={metric} title="busy workers / max workers" aria-label={`${l.label} workers ${busy} of ${max} busy`}>
                  pool {busy}/{max}
                </span>
              </div>
            )}
            {l.tasks.map((t) => (
              <Card
                key={t.id}
                task={t}
                label={cardLabel(t)}
                tokens={tokensByTask[t.id] ?? 0}
                onClick={onOpenCard}
              />
            ))}
          </div>
        );
      })}
    </div>
  );
}
