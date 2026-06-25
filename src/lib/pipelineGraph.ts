import type { Pipeline } from "../ipc/pipeline";

/// Static read-only graph model for the pipeline (audit Decision 2). Pure layout:
/// no drag, no editing. Computes role-typed nodes placed in columns by a simple
/// longest-path-from-source ranking, plus the route edges that honour
/// fork/join/gate wiring. The SVG renderer (PipelineGraph) is dumb; all the graph
/// reasoning lives here so it can be unit-tested.

export type NodeRole = "writer" | "reviewer" | "impl" | "gate" | "fork" | "join" | "escalation";

/// Route vocabulary (vet F3). A producer source emits a single **hand-off**; a
/// reviewer/gate source emits the verdict triple **approve · revise · reject**.
/// The off-language "forward"/"escalate" edge words are retired.
export type RouteKind = "hand-off" | "approve" | "revise" | "reject";

/// Back-compat alias: prefer `RouteKind`. (Kept so older imports still type-check
/// during the transition; the values now match RouteKind.)
export type EdgeKind = RouteKind;

export interface GraphNode {
  id: string;
  label: string;
  role: NodeRole;
  col: number;
  row: number;
  /// Fork-nesting depth of the node's owning lane span (0 = not inside any
  /// nested fork; a top-level fork's lane members are depth 1; a fork reached
  /// inside another fork's lane raises its members to depth 2; max 3). Mirrors
  /// the backend validator's MAX_NESTING_DEPTH walk (validate.rs).
  depth: number;
}

export interface GraphEdge {
  from: string;
  to: string;
  kind: RouteKind;
}

export interface PipelineGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  cols: number;
  rows: number;
}

/// Infer a team's *visual* role from its id/name (a display concern, vet F8).
/// Reviewer/gate-feeders -> reviewer (purple); impl/build/code -> impl (orange);
/// everything else is a writer (blue). DESIGN.md §Pipeline editor.
function teamRole(idOrName: string): NodeRole {
  const s = idOrName.toLowerCase();
  if (/review|critic|qa|verify|check/.test(s)) return "reviewer";
  if (/impl|build|code|dev|engineer|write-code|exec/.test(s)) return "impl";
  return "writer";
}

/// Shared producer-vs-reviewer inference (vet F8). The verdict-vs-hand-off edge
/// distinction depends on this *behavioural* role (distinct from the visual
/// `NodeRole`). Reads the explicit `role` field when present (chunk ①), and falls
/// back to the name regex only when it is absent — so a mid-build draft that has
/// not set the field still routes sensibly.
export function inferTeamRole(team: { role?: "producer" | "reviewer"; id: string; name?: string }): "producer" | "reviewer" {
  if (team.role) return team.role;
  return teamRole(team.name || team.id) === "reviewer" ? "reviewer" : "producer";
}

/// Max fork nesting depth, mirroring the backend (validate.rs MAX_NESTING_DEPTH).
/// A top-level fork is depth 1; a fork reached inside another fork's lane is 2; etc.
export const MAX_NESTING_DEPTH = 3;

/// The single shared lane-walk over a pipeline, replicating the backend validator
/// (`validate.rs::check_lane_reachable`). Built once per call so both
/// `forkNestingDepths` and `nestingGroups` derive the fragile join-pairing the
/// SAME way — there is intentionally no second, simplified matcher. Hops: a team
/// follows `outputs.on_approve`; a gate follows `downstream`; a fork follows its
/// paired join's `downstream`. Bounded by node count so it terminates on any
/// (even malformed) input.
function laneWalk(pipeline: Pipeline) {
  const teams = new Map(pipeline.teams.map((t) => [t.id, t]));
  const gates = new Map(pipeline.gates.map((g) => [g.id, g]));
  const forks = new Map(pipeline.forks.map((f) => [f.id, f]));
  const joins = pipeline.joins;
  const bound =
    pipeline.teams.length + pipeline.gates.length + pipeline.forks.length + pipeline.joins.length + 1;

  // Does walking forward from `entry` reach `target` before the lane ends?
  function laneReaches(entry: string, target: string): boolean {
    let cur = entry;
    for (let i = 0; i <= bound; i++) {
      if (cur === target) return true;
      if (teams.has(cur)) {
        const next = teams.get(cur)!.outputs?.on_approve;
        if (!next) return false;
        cur = next;
      } else if (gates.has(cur)) {
        cur = gates.get(cur)!.downstream;
      } else if (forks.has(cur)) {
        const ds = pairedJoinDownstream(cur);
        if (ds === undefined) return false;
        cur = ds;
      } else {
        return false;
      }
    }
    return false;
  }

  // The join all of a fork's lanes can reach (the derived pairing, validate.rs:101).
  function pairedJoin(forkId: string) {
    const f = forks.get(forkId);
    if (!f) return undefined;
    return joins.find((jn) => f.lanes.every((lane) => laneReaches(lane, jn.id)));
  }

  function pairedJoinDownstream(forkId: string): string | undefined {
    return pairedJoin(forkId)?.downstream;
  }

  return { teams, gates, forks, joins, bound, laneReaches, pairedJoin, pairedJoinDownstream };
}

/// fork id -> nesting depth (1-based). Derived purely from the lane-walk the
/// backend validator performs: a fork's depth is 1 plus the depth of the
/// deepest fork whose lane reaches it. We compute this by walking every fork's
/// lanes (depth d) and recording any fork hop encountered at depth d+1, taking
/// the max. Bounded by node count so it terminates on any (even malformed) input.
export function forkNestingDepths(pipeline: Pipeline): Map<string, number> {
  const { teams, gates, forks, bound, pairedJoinDownstream } = laneWalk(pipeline);

  const depth = new Map<string, number>();
  for (const f of pipeline.forks) depth.set(f.id, 0);

  // Walk a lane at depth d, raising the depth of any fork hop to d+1 (capped).
  function walkLane(entry: string, d: number, seen: Set<string>): void {
    let cur = entry;
    for (let i = 0; i <= bound; i++) {
      if (teams.has(cur)) {
        const next = teams.get(cur)!.outputs?.on_approve;
        if (!next) return;
        cur = next;
      } else if (gates.has(cur)) {
        cur = gates.get(cur)!.downstream;
      } else if (forks.has(cur)) {
        const childDepth = Math.min(d + 1, MAX_NESTING_DEPTH);
        if ((depth.get(cur) ?? 0) < childDepth) depth.set(cur, childDepth);
        if (!seen.has(cur)) {
          seen.add(cur);
          for (const lane of forks.get(cur)!.lanes) walkLane(lane, childDepth, seen);
        }
        const ds = pairedJoinDownstream(cur);
        if (ds === undefined) return;
        cur = ds;
      } else {
        return; // join (the target), escalation, or unknown — lane ends here.
      }
    }
  }

  // Seed: every fork not reachable inside another fork's lane is top-level (depth 1).
  // We discover nesting by walking each top-level fork; a fork only raised by a
  // walk keeps that. Run a settle loop bounded by fork count for transitive depth.
  for (let pass = 0; pass < pipeline.forks.length + 1; pass++) {
    for (const f of pipeline.forks) {
      const d = depth.get(f.id) === 0 ? 1 : depth.get(f.id)!;
      if (depth.get(f.id) === 0) depth.set(f.id, 1);
      for (const lane of f.lanes) walkLane(lane, d, new Set([f.id]));
    }
  }

  return depth;
}

/// A nested fork (depth >= 2) and the set of node ids that belong to its lane
/// span (the fork node, every node reachable in its lanes up to and including
/// its paired join). The renderer draws a labelled containment box around these.
export interface NestingGroup {
  forkId: string;
  depth: number;
  memberIds: string[];
}

export function nestingGroups(pipeline: Pipeline): NestingGroup[] {
  const depths = forkNestingDepths(pipeline);
  const { teams, gates, forks, bound, pairedJoin, pairedJoinDownstream } = laneWalk(pipeline);

  // Collect every node id reachable inside `forkId`'s lanes, up to its join.
  // Uses the SAME shared pairing as the depth walk (no simplified matcher).
  const collect = (forkId: string): string[] => {
    const f = forks.get(forkId);
    if (!f) return [forkId];
    const paired = pairedJoin(forkId);
    const members = new Set<string>([forkId]);
    if (paired) members.add(paired.id);
    for (const lane of f.lanes) {
      let cur = lane;
      for (let i = 0; i <= bound; i++) {
        members.add(cur);
        if (paired && cur === paired.id) break;
        if (teams.has(cur)) {
          const n = teams.get(cur)!.outputs?.on_approve;
          if (!n) break;
          cur = n;
        } else if (gates.has(cur)) {
          cur = gates.get(cur)!.downstream;
        } else if (forks.has(cur)) {
          for (const id of collect(cur)) members.add(id);
          const ds = pairedJoinDownstream(cur);
          if (ds === undefined) break;
          cur = ds;
        } else {
          break;
        }
      }
    }
    return [...members];
  };

  const groups: NestingGroup[] = [];
  for (const f of pipeline.forks) {
    const d = depths.get(f.id) ?? 0;
    if (d >= 2) groups.push({ forkId: f.id, depth: d, memberIds: collect(f.id) });
  }
  return groups;
}

export function buildPipelineGraph(pipeline: Pipeline): PipelineGraph {
  const nodes = new Map<string, GraphNode>();
  const addNode = (id: string, label: string, role: NodeRole) => {
    if (!nodes.has(id)) nodes.set(id, { id, label, role, col: 0, row: 0, depth: 0 });
  };

  for (const t of pipeline.teams) addNode(t.id, t.name || t.id, teamRole(t.name || t.id));
  for (const g of pipeline.gates) addNode(g.id, g.label || g.id, "gate");
  for (const f of pipeline.forks) addNode(f.id, f.id, "fork");
  for (const j of pipeline.joins) addNode(j.id, j.id, "join");
  for (const e of pipeline.escalations) addNode(e.id, e.id, "escalation");

  const edges: GraphEdge[] = [];
  const adj = new Map<string, string[]>(); // forward adjacency for ranking
  // hand-off + approve advance the flow (used for column ranking); revise/reject
  // are returns/sinks and never rank forward.
  const pushEdge = (from: string, to: string, kind: RouteKind) => {
    if (!nodes.has(from) || !nodes.has(to)) return;
    edges.push({ from, to, kind });
    if (kind === "hand-off" || kind === "approve") {
      adj.set(from, [...(adj.get(from) ?? []), to]);
    }
  };

  for (const t of pipeline.teams) {
    const o = t.outputs ?? {};
    // Role-aware (vet F3): a producer's on_approve is a hand-off, not a verdict;
    // a reviewer's is the approve leg of the verdict triple.
    const handoff = inferTeamRole(t) === "producer";
    if (o.on_approve) pushEdge(t.id, o.on_approve, handoff ? "hand-off" : "approve");
    if (o.on_revise) pushEdge(t.id, o.on_revise, "revise");
    if (o.on_reject) pushEdge(t.id, o.on_reject, "reject");
  }
  for (const g of pipeline.gates) {
    // A gate is a reviewer-shaped node; its single downstream is the approve leg.
    if (g.downstream) pushEdge(g.id, g.downstream, "approve");
  }
  for (const f of pipeline.forks) {
    for (const lane of f.lanes) pushEdge(f.id, lane, "hand-off");
  }
  for (const j of pipeline.joins) {
    for (const w of j.waits_for) pushEdge(w, j.id, "hand-off");
    if (j.downstream) pushEdge(j.id, j.downstream, "hand-off");
  }

  // Column = longest forward path from any source (nodes with no forward
  // in-edge). Iterative relaxation; cycle-safe via a bounded pass count.
  const indeg = new Map<string, number>();
  for (const id of nodes.keys()) indeg.set(id, 0);
  for (const [, tos] of adj) for (const to of tos) indeg.set(to, (indeg.get(to) ?? 0) + 1);

  const col = new Map<string, number>();
  for (const id of nodes.keys()) col.set(id, 0);
  const passes = nodes.size + 1;
  for (let p = 0; p < passes; p++) {
    let changed = false;
    for (const [from, tos] of adj) {
      const fc = col.get(from) ?? 0;
      for (const to of tos) {
        if ((col.get(to) ?? 0) < fc + 1) {
          col.set(to, fc + 1);
          changed = true;
        }
      }
    }
    if (!changed) break;
  }

  // Row = order of appearance within a column (stable, deterministic).
  const rowCounter = new Map<number, number>();
  let maxCol = 0;
  for (const node of nodes.values()) {
    const c = col.get(node.id) ?? 0;
    const r = rowCounter.get(c) ?? 0;
    node.col = c;
    node.row = r;
    rowCounter.set(c, r + 1);
    if (c > maxCol) maxCol = c;
  }

  let maxRow = 0;
  for (const r of rowCounter.values()) if (r > maxRow) maxRow = r;

  // Hierarchical depth (AU2): a fork node carries its own nesting depth (1+);
  // a node inside a nested fork's lane span inherits that fork's depth. A node
  // in several groups takes the deepest (max), capped at MAX_NESTING_DEPTH.
  const forkDepth = forkNestingDepths(pipeline);
  for (const node of nodes.values()) {
    const fd = forkDepth.get(node.id);
    if (fd !== undefined) node.depth = Math.max(node.depth, fd); // fork node carries its own depth (1+)
  }
  for (const grp of nestingGroups(pipeline)) {
    for (const id of grp.memberIds) {
      const n = nodes.get(id);
      if (n) n.depth = Math.max(n.depth, grp.depth);
    }
  }

  return { nodes: [...nodes.values()], edges, cols: maxCol + 1, rows: maxRow };
}

/// Border colour per node role (DESIGN.md §Pipeline editor node table). Returns a
/// CSS value (token or the spec'd literal for writer/reviewer/impl).
export function nodeBorderColor(role: NodeRole): string {
  switch (role) {
    case "writer": return "oklch(40% 0.07 240)";
    case "reviewer": return "oklch(45% 0.09 300)";
    case "impl": return "oklch(45% 0.10 50)";
    case "gate": return "var(--accent-bd)";
    case "escalation": return "var(--danger)";
    case "fork":
    case "join": return "var(--border-2)";
  }
}
