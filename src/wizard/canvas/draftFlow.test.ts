import { describe, expect, it } from "vitest";
import { emptyDraft, addTeam, setTeamRole } from "../draft";
import { connect } from "./mutations";
import { draftToFlow, reconcile } from "./draftFlow";
import type { DraftPipeline } from "../../ipc/pipeline";

describe("draftToFlow — nodes", () => {
  it("empty draft → no nodes, no edges, never throws", () => {
    const { nodes, edges } = draftToFlow(emptyDraft());
    expect(nodes).toEqual([]);
    expect(edges).toEqual([]);
  });

  it("emits one node per team/gate/fork/join/escalation with its kind as type", () => {
    let d = addTeam(emptyDraft(), "a", "A");
    d = {
      ...d,
      gates: [{ id: "g1", label: "G", downstream: "" }],
      forks: [{ id: "fork-1", lanes: [] }],
      joins: [{ id: "join-1", waits_for: [], downstream: "" }],
      escalations: [{ id: "needs-human", triggers: [] }],
    };
    const { nodes } = draftToFlow(d);
    const byId = Object.fromEntries(nodes.map((n) => [n.id, n.type]));
    expect(byId).toEqual({ a: "team", g1: "gate", "fork-1": "fork", "join-1": "join", "needs-human": "escalation" });
  });

  it("flags a producer team with no prompt as a warning in node data", () => {
    const d = addTeam(emptyDraft(), "a", "A");
    const { nodes } = draftToFlow(d);
    const node = nodes.find((n) => n.id === "a")!;
    expect(node.data.warnings.length).toBeGreaterThan(0);
  });

  it("a team with a prompt has no missing-prompt warning", () => {
    let d = addTeam(emptyDraft(), "a", "A");
    d = { ...d, teams: d.teams.map((t) => ({ ...t, prompt_body: "do the thing" })) };
    const { nodes } = draftToFlow(d);
    expect(nodes.find((n) => n.id === "a")!.data.warnings).toEqual([]);
  });
});

describe("draftToFlow — edges (role-aware)", () => {
  it("a producer hand-off edge carries kind hand-off (now via the input store)", () => {
    let d = addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B");
    d = connect(d, "a", "b");
    const { edges } = draftToFlow(d);
    // G1: the hand-off is rerouted through b's input store.
    const e = edges.find((e) => e.source === "a" && e.target === "store:b")!;
    expect(e.data.kind).toBe("hand-off");
  });

  it("a reviewer emits approve/revise/reject edges", () => {
    let d = addTeam(addTeam(addTeam(emptyDraft(), "rev", "Rev"), "b", "B"), "c", "C");
    d = setTeamRole(d, "rev", "reviewer");
    d = connect(d, "rev", "b", "approve");
    d = connect(d, "rev", "c", "revise");
    const { edges } = draftToFlow(d);
    const kinds = edges.filter((e) => e.source === "rev").map((e) => e.data.kind).sort();
    expect(kinds).toEqual(["approve", "revise"]);
  });

  it("a dangling route (target node absent) is flagged on the edge, never crashes", () => {
    let d = addTeam(emptyDraft(), "a", "A");
    d = { ...d, teams: d.teams.map((t) => ({ ...t, outputs: { on_approve: "ghost" } })) };
    const { edges } = draftToFlow(d);
    const e = edges.find((e) => e.source === "a")!;
    expect(e.data.dangling).toBe(true);
  });

  it("each edge has a unique id", () => {
    let d = addTeam(addTeam(addTeam(emptyDraft(), "rev", "Rev"), "b", "B"), "c", "C");
    d = setTeamRole(d, "rev", "reviewer");
    d = connect(d, "rev", "b", "approve");
    d = connect(d, "rev", "c", "reject");
    const { edges } = draftToFlow(d);
    const ids = edges.map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});

describe("draftToFlow — store nodes (G1)", () => {
  function withForward(): DraftPipeline {
    // a (producer) hand-off → b (producer); b is a consumer with an inbound edge.
    let d = addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B");
    d = connect(d, "a", "b");
    return d;
  }

  it("synthesizes a store node before a team with an inbound forward edge", () => {
    const { nodes } = draftToFlow(withForward());
    const store = nodes.find((n) => n.id === "store:b");
    expect(store).toBeDefined();
    expect(store!.type).toBe("store" as never);
  });

  it("the source team (no inbound) has no input store", () => {
    const { nodes } = draftToFlow(withForward());
    expect(nodes.find((n) => n.id === "store:a")).toBeUndefined();
  });

  it("reroutes producer → store → team (the original edge now targets the store)", () => {
    const { edges } = draftToFlow(withForward());
    // The original a → b edge is gone; a → store:b and store:b → b exist.
    expect(edges.find((e) => e.source === "a" && e.target === "b")).toBeUndefined();
    expect(edges.find((e) => e.source === "a" && e.target === "store:b")).toBeDefined();
    expect(edges.find((e) => e.source === "store:b" && e.target === "b")).toBeDefined();
  });

  it("the store node carries the owning team's capacity + team id", () => {
    let d = withForward();
    d = { ...d, teams: d.teams.map((t) => (t.id === "b" ? { ...t, store: { capacity: 3 } } : t)) };
    const { nodes } = draftToFlow(d);
    const store = nodes.find((n) => n.id === "store:b")!;
    expect(store.data.capacity).toBe(3);
    expect(store.data.storeTeamId).toBe("b");
  });

  it("N producer→team edges all route through the one team store", () => {
    let d = addTeam(addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B"), "c", "C");
    d = connect(d, "a", "c");
    d = connect(d, "b", "c");
    const { edges, nodes } = draftToFlow(d);
    expect(nodes.filter((n) => n.id === "store:c")).toHaveLength(1);
    expect(edges.find((e) => e.source === "a" && e.target === "store:c")).toBeDefined();
    expect(edges.find((e) => e.source === "b" && e.target === "store:c")).toBeDefined();
    expect(edges.filter((e) => e.source === "store:c" && e.target === "c")).toHaveLength(1);
  });

  it("revise/reject return edges are NOT routed through the store", () => {
    let d = addTeam(addTeam(addTeam(emptyDraft(), "rev", "Rev"), "w", "W"), "esc", "Esc");
    d = setTeamRole(d, "rev", "reviewer");
    d = connect(d, "rev", "w", "revise");
    const { edges } = draftToFlow(d);
    // revise goes straight to the writer, not through store:w.
    expect(edges.find((e) => e.source === "rev" && e.target === "w" && e.data.kind === "revise")).toBeDefined();
  });

  it("is idempotent — the projection is stable for the same draft", () => {
    const d = withForward();
    expect(draftToFlow(d)).toEqual(draftToFlow(d));
  });
});

describe("reconcile — positions", () => {
  function flowOf(d: DraftPipeline) {
    return draftToFlow(d);
  }

  it("auto-lays-out every node when there are no prior positions", () => {
    let d = addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B");
    d = connect(d, "a", "b");
    const placed = reconcile(flowOf(d).nodes, flowOf(d).edges, {});
    expect(placed.every((n) => Number.isFinite(n.position.x) && Number.isFinite(n.position.y))).toBe(true);
  });

  it("preserves an existing position and only lays out the new node", () => {
    let d = addTeam(emptyDraft(), "a", "A");
    const positions = { a: { x: 999, y: 111 } };
    // add a second node — 'a' must keep its dragged position
    d = addTeam(d, "b", "B");
    d = connect(d, "a", "b");
    const flow = flowOf(d);
    const placed = reconcile(flow.nodes, flow.edges, positions);
    const a = placed.find((n) => n.id === "a")!;
    const b = placed.find((n) => n.id === "b")!;
    expect(a.position).toEqual({ x: 999, y: 111 });
    expect(Number.isFinite(b.position.x)).toBe(true);
  });
});
