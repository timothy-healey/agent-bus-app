import { describe, expect, it } from "vitest";
import { buildPipelineGraph, nodeBorderColor, inferTeamRole, forkNestingDepths, nestingGroups } from "./pipelineGraph";
import type { Pipeline } from "../ipc/pipeline";

// A team builder local to these tests (same shape as the existing one above).
function tm(id: string, on_approve?: string): Pipeline["teams"][number] {
  return {
    id, name: id, prompt: "", scope: { reads: [], writes: [], tools: [] },
    outputs: on_approve ? { on_approve } : {}, workers: { min: 1, max: 1 },
    role: "producer", store: { capacity: 8 },
  };
}

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

  it("stamps a nesting depth on nodes inside a nested fork's lane span", () => {
    const p = pipe({
      schema_version: 2,
      teams: [tm("ia", "inner-join"), tm("ib", "inner-join"), tm("lb", "outer-join"), tm("after")],
      forks: [
        { id: "outer", lanes: ["inner", "lb"] },
        { id: "inner", lanes: ["ia", "ib"] },
      ],
      joins: [
        { id: "inner-join", waits_for: ["ia", "ib"], downstream: "outer-join" },
        { id: "outer-join", waits_for: ["inner", "lb"], downstream: "after" },
      ],
    });
    const g = buildPipelineGraph(p);
    const depth = (id: string) => g.nodes.find((n) => n.id === id)!.depth;
    expect(depth("outer")).toBe(1);   // top-level fork
    expect(depth("inner")).toBe(2);   // nested fork
    expect(depth("ia")).toBe(2);      // node inside the nested lane span
    expect(depth("after")).toBe(0);   // downstream of everything, not nested
  });
});

describe("forkNestingDepths", () => {
  it("a single top-level fork is depth 1", () => {
    const p = pipe({
      schema_version: 2,
      teams: [tm("la", "outer-join"), tm("lb", "outer-join"), tm("after")],
      forks: [{ id: "outer", lanes: ["la", "lb"] }],
      joins: [{ id: "outer-join", waits_for: ["la", "lb"], downstream: "after" }],
    });
    const d = forkNestingDepths(p);
    expect(d.get("outer")).toBe(1);
  });

  it("a fork reached inside another fork's lane is depth 2", () => {
    // outer lane "la" enters the INNER fork directly; inner lanes reach inner-join,
    // whose downstream "mid" reaches outer-join. Mirrors validate.rs lane-walk.
    const p = pipe({
      schema_version: 2,
      teams: [tm("ia", "inner-join"), tm("ib", "inner-join"), tm("mid", "outer-join"), tm("lb", "outer-join"), tm("after")],
      forks: [
        { id: "outer", lanes: ["la", "lb"] },
        { id: "inner", lanes: ["ia", "ib"] },
      ],
      joins: [
        { id: "inner-join", waits_for: ["ia", "ib"], downstream: "mid" },
        { id: "outer-join", waits_for: ["la", "lb"], downstream: "after" },
      ],
    });
    // "la" IS the inner fork's id (the lane entry is the nested fork itself).
    p.forks[0].lanes = ["inner", "lb"];
    const d = forkNestingDepths(p);
    expect(d.get("outer")).toBe(1);
    expect(d.get("inner")).toBe(2);
  });

  it("resolves nesting through a team hop before the nested fork", () => {
    // outer lane "oa" is a TEAM that hands off (on_approve) to the inner fork —
    // exercising the team->on_approve->fork path of the lane-walk, not just a
    // direct fork-as-lane-entry. Mirrors validate.rs's multi-hop reachability.
    const p = pipe({
      schema_version: 2,
      teams: [
        tm("oa", "inner"),
        tm("ia", "inner-join"),
        tm("ib", "inner-join"),
        tm("mid", "outer-join"),
        tm("lb", "outer-join"),
        tm("after"),
      ],
      forks: [
        { id: "outer", lanes: ["oa", "lb"] },
        { id: "inner", lanes: ["ia", "ib"] },
      ],
      joins: [
        { id: "inner-join", waits_for: ["ia", "ib"], downstream: "mid" },
        { id: "outer-join", waits_for: ["oa", "lb"], downstream: "after" },
      ],
    });
    const d = forkNestingDepths(p);
    expect(d.get("outer")).toBe(1);
    expect(d.get("inner")).toBe(2); // reached via oa's on_approve, one hop in
  });

  it("caps nesting depth at MAX_NESTING_DEPTH (3) even for a deeper chain", () => {
    // f1 lane -> f2 lane -> f3 lane -> f4 (would be depth 4, must clamp to 3).
    const p = pipe({
      schema_version: 2,
      teams: [
        tm("a3", "j3"), tm("b3", "j3"),
        tm("b2", "j2"), tm("b1", "j1"), tm("after"),
      ],
      forks: [
        { id: "f1", lanes: ["f2", "b1"] },
        { id: "f2", lanes: ["f3", "b2"] },
        { id: "f3", lanes: ["f4", "b3"] },
        { id: "f4", lanes: ["a3", "b3"] },
      ],
      joins: [
        { id: "j4", waits_for: ["a3", "b3"], downstream: "j3" },
        { id: "j3", waits_for: ["f4", "b3"], downstream: "j2" },
        { id: "j2", waits_for: ["f3", "b2"], downstream: "j1" },
        { id: "j1", waits_for: ["f2", "b1"], downstream: "after" },
      ],
    });
    const d = forkNestingDepths(p);
    for (const v of d.values()) expect(v).toBeLessThanOrEqual(3);
    expect(d.get("f4")).toBe(3); // clamped, not 4
  });
});

describe("nestingGroups", () => {
  it("returns one group per nested (depth>=2) fork, listing the fork + its lane members", () => {
    const p = pipe({
      schema_version: 2,
      teams: [tm("ia", "inner-join"), tm("ib", "inner-join"), tm("lb", "outer-join"), tm("after")],
      forks: [
        { id: "outer", lanes: ["inner", "lb"] },
        { id: "inner", lanes: ["ia", "ib"] },
      ],
      joins: [
        { id: "inner-join", waits_for: ["ia", "ib"], downstream: "outer-join" },
        { id: "outer-join", waits_for: ["inner", "lb"], downstream: "after" },
      ],
    });
    const groups = nestingGroups(p);
    // top-level "outer" is NOT a group; only the nested "inner" fork is.
    expect(groups.map((g) => g.forkId)).toEqual(["inner"]);
    const inner = groups[0];
    expect(inner.depth).toBe(2);
    // members include the fork node, its lane entries and the paired join.
    expect(inner.memberIds).toContain("inner");
    expect(inner.memberIds).toContain("ia");
    expect(inner.memberIds).toContain("ib");
    expect(inner.memberIds).toContain("inner-join");
  });

  it("returns no groups when no fork is nested", () => {
    const p = pipe({
      schema_version: 2,
      teams: [tm("la", "j"), tm("lb", "j"), tm("after")],
      forks: [{ id: "f", lanes: ["la", "lb"] }],
      joins: [{ id: "j", waits_for: ["la", "lb"], downstream: "after" }],
    });
    expect(nestingGroups(p)).toEqual([]);
  });
});
