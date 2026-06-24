import { describe, expect, it } from "vitest";
import { emptyDraft, addTeam, setTeamRole } from "../draft";
import { addNode, connect, setRouteKind, removeNode, removeEdge, NODE_KINDS } from "./mutations";

describe("canvas mutations — addNode", () => {
  it("NODE_KINDS lists the five authoring kinds", () => {
    expect(NODE_KINDS).toEqual(["team", "gate", "fork", "join", "escalation"]);
  });

  it("addNode('team') appends a defaulted team and returns its id", () => {
    const { draft, newId } = addNode(emptyDraft(), "team");
    expect(draft.teams).toHaveLength(1);
    expect(draft.teams[0].id).toBe(newId);
    expect(draft.teams[0].role).toBe("producer");
  });

  it("addNode generates unique ids across repeated adds", () => {
    const a = addNode(emptyDraft(), "team");
    const b = addNode(a.draft, "team");
    expect(a.newId).not.toBe(b.newId);
    expect(b.draft.teams).toHaveLength(2);
  });

  it("addNode('gate') appends a gate", () => {
    const { draft, newId } = addNode(emptyDraft(), "gate");
    expect(draft.gates).toHaveLength(1);
    expect(draft.gates[0].id).toBe(newId);
  });

  it("addNode('escalation') appends an escalation", () => {
    const { draft, newId } = addNode(emptyDraft(), "escalation");
    expect(draft.escalations).toHaveLength(1);
    expect(draft.escalations[0].id).toBe(newId);
  });

  it("addNode('fork') creates a paired fork + join (W4 primitive)", () => {
    const { draft, newId } = addNode(emptyDraft(), "fork");
    expect(draft.forks).toHaveLength(1);
    expect(draft.joins).toHaveLength(1);
    expect(draft.forks[0].id).toBe(newId);
  });

  it("addNode('join') is the same paired primitive as fork", () => {
    const { draft } = addNode(emptyDraft(), "join");
    expect(draft.forks).toHaveLength(1);
    expect(draft.joins).toHaveLength(1);
  });
});

describe("canvas mutations — connect (role-aware)", () => {
  it("producer source → on_approve hand-off", () => {
    let d = addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B");
    d = connect(d, "a", "b");
    expect(d.teams.find((t) => t.id === "a")?.outputs.on_approve).toBe("b");
  });

  it("reviewer source default route is approve", () => {
    let d = addTeam(addTeam(emptyDraft(), "rev", "Rev"), "b", "B");
    d = setTeamRole(d, "rev", "reviewer");
    d = connect(d, "rev", "b");
    expect(d.teams.find((t) => t.id === "rev")?.outputs.on_approve).toBe("b");
  });

  it("reviewer source honours an explicit routeKind (revise / reject)", () => {
    let d = addTeam(addTeam(addTeam(emptyDraft(), "rev", "Rev"), "b", "B"), "c", "C");
    d = setTeamRole(d, "rev", "reviewer");
    d = connect(d, "rev", "b", "revise");
    d = connect(d, "rev", "c", "reject");
    const rev = d.teams.find((t) => t.id === "rev")!;
    expect(rev.outputs.on_revise).toBe("b");
    expect(rev.outputs.on_reject).toBe("c");
  });

  it("gate source → downstream", () => {
    let d = addTeam(emptyDraft(), "b", "B");
    d = { ...d, gates: [{ id: "gate-1", label: "G", downstream: "" }] };
    d = connect(d, "gate-1", "b");
    expect(d.gates[0].downstream).toBe("b");
  });

  it("fork source → adds the target as a lane (no dup)", () => {
    let d = addTeam(addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B"), "c", "C");
    d = { ...d, forks: [{ id: "fork-1", lanes: ["a"] }], joins: [{ id: "join-1", waits_for: ["a"], downstream: "" }] };
    d = connect(d, "fork-1", "b");
    expect(d.forks[0].lanes).toEqual(["a", "b"]);
    d = connect(d, "fork-1", "b"); // idempotent
    expect(d.forks[0].lanes).toEqual(["a", "b"]);
  });

  it("connect is a no-op for unknown source", () => {
    const d = addTeam(emptyDraft(), "a", "A");
    expect(connect(d, "nope", "a")).toBe(d);
  });
});

describe("canvas mutations — setRouteKind", () => {
  it("moves an existing reviewer edge from approve to revise", () => {
    let d = addTeam(addTeam(emptyDraft(), "rev", "Rev"), "b", "B");
    d = setTeamRole(d, "rev", "reviewer");
    d = connect(d, "rev", "b"); // approve -> b
    d = setRouteKind(d, "rev", "b", "revise");
    const rev = d.teams.find((t) => t.id === "rev")!;
    expect(rev.outputs.on_approve == null).toBe(true);
    expect(rev.outputs.on_revise).toBe("b");
  });
});

describe("canvas mutations — removeEdge", () => {
  it("clears the matching route slot", () => {
    let d = addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B");
    d = connect(d, "a", "b"); // hand-off
    d = removeEdge(d, "a", "b");
    expect(d.teams.find((t) => t.id === "a")?.outputs.on_approve == null).toBe(true);
  });

  it("removes a fork lane", () => {
    let d = { ...emptyDraft(), forks: [{ id: "fork-1", lanes: ["a", "b"] }], joins: [{ id: "join-1", waits_for: ["a", "b"], downstream: "" }] };
    d = removeEdge(d, "fork-1", "b");
    expect(d.forks[0].lanes).toEqual(["a"]);
  });
});

describe("canvas mutations — removeNode clears dangling refs", () => {
  it("removing a team drops it AND clears routes pointing at it", () => {
    let d = addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B");
    d = connect(d, "a", "b"); // a.on_approve -> b
    d = removeNode(d, "b");
    expect(d.teams.map((t) => t.id)).toEqual(["a"]);
    expect(d.teams.find((t) => t.id === "a")?.outputs.on_approve == null).toBe(true);
  });

  it("removing a node clears references in gates, fork lanes and join waits_for", () => {
    let d = addTeam(addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B"), "c", "C");
    d = {
      ...d,
      gates: [{ id: "g1", label: "G", downstream: "c" }],
      forks: [{ id: "fork-1", lanes: ["a", "c"] }],
      joins: [{ id: "join-1", waits_for: ["a", "c"], downstream: "c" }],
    };
    d = removeNode(d, "c");
    expect(d.gates[0].downstream).toBe("");
    expect(d.forks[0].lanes).toEqual(["a"]);
    expect(d.joins[0].waits_for).toEqual(["a"]);
    expect(d.joins[0].downstream).toBe("");
  });

  it("removing a fork node drops its paired join too", () => {
    let d = { ...emptyDraft(), forks: [{ id: "fork-1", lanes: ["a", "b"] }], joins: [{ id: "join-1", waits_for: ["a", "b"], downstream: "" }] };
    d = removeNode(d, "fork-1");
    expect(d.forks).toHaveLength(0);
    expect(d.joins).toHaveLength(0);
  });
});
