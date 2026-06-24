import { describe, expect, it } from "vitest";
import { buildPipelineGraph, nodeBorderColor, inferTeamRole } from "./pipelineGraph";
import type { Pipeline } from "../ipc/pipeline";

function pipe(over: Partial<Pipeline> = {}): Pipeline {
  return {
    id: "p", name: "P", description: "", schema_version: 1,
    teams: [], gates: [], escalations: [], forks: [], joins: [], ...over,
  };
}

describe("buildPipelineGraph", () => {
  it("creates a node per team, gate, fork, join and escalation", () => {
    const g = buildPipelineGraph(
      pipe({
        teams: [{ id: "writer", name: "Writer", prompt: "", scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { min: 1, max: 1 }, role: "producer", store: { capacity: 8 } }],
        gates: [{ id: "gate-1", label: "Gate 1", downstream: "impl" }],
        escalations: [{ id: "needs-human", triggers: [] }],
        forks: [{ id: "fork-1", lanes: ["a", "b"] }],
        joins: [{ id: "join-1", waits_for: ["a", "b"], downstream: "after" }],
      }),
    );
    const ids = g.nodes.map((n) => n.id).sort();
    expect(ids).toEqual(["fork-1", "gate-1", "join-1", "needs-human", "writer"]);
  });

  it("ranks columns by longest forward path", () => {
    const g = buildPipelineGraph(
      pipe({
        teams: [
          { id: "a", name: "A", prompt: "", scope: { reads: [], writes: [], tools: [] }, outputs: { on_approve: "gate-1" }, workers: { min: 1, max: 1 }, role: "producer", store: { capacity: 8 } },
        ],
        gates: [{ id: "gate-1", label: "G", downstream: "b" }],
      }),
    );
    const col = (id: string) => g.nodes.find((n) => n.id === id)!.col;
    expect(col("a")).toBe(0);
    expect(col("gate-1")).toBe(1);
  });

  it("a reviewer source emits the approve · revise · reject verdict triple", () => {
    const team = (id: string, name: string, role: "producer" | "reviewer", outputs: Record<string, string> = {}) => ({
      id, name, prompt: "", scope: { reads: [], writes: [], tools: [] }, outputs, workers: { min: 1, max: 1 }, role, store: { capacity: 8 },
    });
    const g = buildPipelineGraph(
      pipe({
        teams: [
          team("writer", "Writer", "producer"),
          team("next", "Next", "producer"),
          team("rev", "Reviewer", "reviewer", { on_approve: "next", on_revise: "writer", on_reject: "needs-human" }),
        ],
        escalations: [{ id: "needs-human", triggers: [] }],
      }),
    );
    const kinds = g.edges.filter((e) => e.from === "rev").map((e) => `${e.kind}->${e.to}`).sort();
    expect(kinds).toContain("approve->next");
    expect(kinds).toContain("revise->writer");
    expect(kinds).toContain("reject->needs-human");
  });

  it("a producer source's on_approve is a hand-off, not a verdict (vet F3)", () => {
    const g = buildPipelineGraph(
      pipe({
        teams: [
          { id: "writer", name: "Writer", prompt: "", scope: { reads: [], writes: [], tools: [] }, outputs: { on_approve: "next" }, workers: { min: 1, max: 1 }, role: "producer", store: { capacity: 8 } },
          { id: "next", name: "Next", prompt: "", scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { min: 1, max: 1 }, role: "producer", store: { capacity: 8 } },
        ],
      }),
    );
    const e = g.edges.find((e) => e.from === "writer" && e.to === "next");
    expect(e?.kind).toBe("hand-off");
  });

  it("inferTeamRole reads the explicit role first, then falls back to the name regex", () => {
    // explicit field wins even against a reviewer-shaped name
    expect(inferTeamRole({ id: "spec-review", name: "Spec Review", role: "producer" })).toBe("producer");
    // fallback: no role field, reviewer-shaped name → reviewer
    expect(inferTeamRole({ id: "spec-review", name: "Spec Review" })).toBe("reviewer");
    // fallback: producer-shaped name → producer
    expect(inferTeamRole({ id: "plan", name: "Planner" })).toBe("producer");
  });

  it("infers reviewer/impl/writer roles from team naming", () => {
    expect(nodeBorderColor("gate")).toContain("--accent-bd");
    const g = buildPipelineGraph(
      pipe({
        teams: [
          { id: "spec-review", name: "Spec Review", prompt: "", scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { min: 1, max: 1 }, role: "producer", store: { capacity: 8 } },
          { id: "impl", name: "Implementer", prompt: "", scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { min: 1, max: 1 }, role: "producer", store: { capacity: 8 } },
          { id: "plan", name: "Planner", prompt: "", scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { min: 1, max: 1 }, role: "producer", store: { capacity: 8 } },
        ],
      }),
    );
    const role = (id: string) => g.nodes.find((n) => n.id === id)!.role;
    expect(role("spec-review")).toBe("reviewer");
    expect(role("impl")).toBe("impl");
    expect(role("plan")).toBe("writer");
  });
});
