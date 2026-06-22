import type { Pipeline } from "../ipc/pipeline";
import type { Task } from "../ipc/runtime";

export type LaneKind = "team" | "gate" | "escalation";

export interface Lane {
  id: string;
  label: string;
  kind: LaneKind;
  tasks: Task[];
}

/// Build board lanes in the pipeline's declared node order: teams first
/// (declaration order), then gates, then escalations. Each task is dropped
/// into the lane matching its `current_stage`; tasks at an unknown stage are
/// omitted rather than crashing the board.
export function buildLanes(pipeline: Pipeline, tasks: Task[]): Lane[] {
  const lanes: Lane[] = [];
  for (const t of pipeline.teams) {
    lanes.push({ id: t.id, label: t.name, kind: "team", tasks: [] });
  }
  for (const g of pipeline.gates) {
    lanes.push({ id: g.id, label: g.label, kind: "gate", tasks: [] });
  }
  for (const e of pipeline.escalations) {
    lanes.push({ id: e.id, label: e.id, kind: "escalation", tasks: [] });
  }

  const byId = new Map(lanes.map((l) => [l.id, l]));
  for (const task of tasks) {
    byId.get(task.current_stage)?.tasks.push(task);
  }
  return lanes;
}
