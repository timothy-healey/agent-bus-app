import { describe, expect, it } from "vitest";
import { filterTasks } from "./listFilter";
import type { Task } from "../ipc/runtime";

function t(over: Partial<Task>): Task {
  return {
    id: "T-1", project_id: "p", pipeline: "pipe", topic: "Bulk write",
    target_repo: null, target_scope: null, current_stage: "research",
    state: "queued", attempts: 1, parent_artifact: null, review_artifact: null,
    created_at: 0, updated_at: 0, ...over,
  };
}

describe("filterTasks", () => {
  const tasks = [
    t({ id: "T-1", state: "running", topic: "alpha bulk" }),
    t({ id: "T-2", state: "gated", topic: "beta gate" }),
    t({ id: "T-3", state: "needs_human", topic: "gamma" }),
    t({ id: "T-4", state: "revising", topic: "delta" }),
    t({ id: "T-5", state: "braked", topic: "epsilon" }),
  ];

  it("all returns everything", () => {
    expect(filterTasks(tasks, "all", "", {}).length).toBe(5);
  });

  it("needs you returns gated + needs_human", () => {
    const r = filterTasks(tasks, "needs you", "", {}).map((x) => x.id);
    expect(r).toEqual(["T-2", "T-3"]);
  });

  it("running pill filters by state", () => {
    expect(filterTasks(tasks, "running", "", {}).map((x) => x.id)).toEqual(["T-1"]);
  });

  it("revising and braked pills filter by state", () => {
    expect(filterTasks(tasks, "revising", "", {}).map((x) => x.id)).toEqual(["T-4"]);
    expect(filterTasks(tasks, "braked", "", {}).map((x) => x.id)).toEqual(["T-5"]);
  });

  it("cost > 100k filters by tokens map", () => {
    const tokens = { "T-1": 150000, "T-2": 50000 };
    expect(filterTasks(tasks, "cost > 100k", "", tokens).map((x) => x.id)).toEqual(["T-1"]);
  });

  it("search matches id or topic, case-insensitive, within a pill", () => {
    expect(filterTasks(tasks, "all", "BETA", {}).map((x) => x.id)).toEqual(["T-2"]);
    expect(filterTasks(tasks, "all", "t-3", {}).map((x) => x.id)).toEqual(["T-3"]);
  });
});
