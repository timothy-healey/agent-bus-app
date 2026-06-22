import { describe, expect, it } from "vitest";
import { buildLanes } from "./lanes";
import type { Pipeline } from "../ipc/pipeline";
import type { Task } from "../ipc/runtime";

function pipeline(): Pipeline {
  return {
    id: "p",
    name: "P",
    description: "",
    schema_version: 1,
    teams: [
      { id: "research", name: "Research" } as any,
      { id: "spec-writers", name: "Spec Writers" } as any,
      { id: "spec-reviewers", name: "Spec Reviewers" } as any,
    ],
    gates: [{ id: "gate-1-spec", label: "Gate 1 — Spec", downstream: "plan-writers" }],
    escalations: [{ id: "needs-human", triggers: [] }],
  };
}

function task(id: string, stage: string, state: Task["state"]): Task {
  return {
    id,
    project_id: "proj",
    pipeline: "p",
    topic: `topic ${id}`,
    target_repo: null,
    target_scope: null,
    current_stage: stage,
    state,
    attempts: 1,
    parent_artifact: null,
    review_artifact: null,
    created_at: 0,
    updated_at: 0,
  };
}

describe("buildLanes", () => {
  it("returns one lane per node in declared order, gates flagged", () => {
    const lanes = buildLanes(pipeline(), []);
    expect(lanes.map((l) => l.id)).toEqual([
      "research",
      "spec-writers",
      "spec-reviewers",
      "gate-1-spec",
      "needs-human",
    ]);
    const gate = lanes.find((l) => l.id === "gate-1-spec")!;
    expect(gate.kind).toBe("gate");
    expect(gate.label).toBe("Gate 1 — Spec");
  });

  it("places each task in its current_stage lane", () => {
    const tasks = [
      task("T-1", "research", "running"),
      task("T-2", "gate-1-spec", "gated"),
    ];
    const lanes = buildLanes(pipeline(), tasks);
    expect(lanes.find((l) => l.id === "research")!.tasks.map((t) => t.id)).toEqual(["T-1"]);
    expect(lanes.find((l) => l.id === "gate-1-spec")!.tasks.map((t) => t.id)).toEqual(["T-2"]);
  });

  it("collects tasks whose stage is unknown into no lane (dropped, not crashed)", () => {
    const lanes = buildLanes(pipeline(), [task("T-9", "ghost-stage", "queued")]);
    const total = lanes.reduce((n, l) => n + l.tasks.length, 0);
    expect(total).toBe(0);
  });
});
