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
  it("a producer hand-off edge carries kind hand-off", () => {
    let d = addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B");
    d = connect(d, "a", "b");
    const { edges } = draftToFlow(d);
    const e = edges.find((e) => e.source === "a" && e.target === "b")!;
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
