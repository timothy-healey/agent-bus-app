import type { Pipeline } from "../ipc/pipeline";

/// Static read-only graph model for the pipeline (audit Decision 2). Pure layout:
/// no drag, no editing. Computes role-typed nodes placed in columns by a simple
/// longest-path-from-source ranking, plus forward / revise / escalate edges that
/// honour fork/join/gate wiring. The SVG renderer (PipelineGraph) is dumb; all
/// the graph reasoning lives here so it can be unit-tested.

export type NodeRole = "writer" | "reviewer" | "impl" | "gate" | "fork" | "join" | "escalation";
export type EdgeKind = "forward" | "revise" | "escalate";

export interface GraphNode {
  id: string;
  label: string;
  role: NodeRole;
  col: number;
  row: number;
}

export interface GraphEdge {
  from: string;
  to: string;
  kind: EdgeKind;
}

export interface PipelineGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  cols: number;
  rows: number;
}

/// Infer a team's visual role from its id/name (teams carry no explicit role in
/// the schema). Reviewer/gate-feeders -> reviewer (purple); impl/build/code ->
/// impl (orange); everything else is a writer (blue). DESIGN.md §Pipeline editor.
function teamRole(idOrName: string): NodeRole {
  const s = idOrName.toLowerCase();
  if (/review|critic|qa|verify|check/.test(s)) return "reviewer";
  if (/impl|build|code|dev|engineer|write-code|exec/.test(s)) return "impl";
  return "writer";
}

export function buildPipelineGraph(pipeline: Pipeline): PipelineGraph {
  const nodes = new Map<string, GraphNode>();
  const addNode = (id: string, label: string, role: NodeRole) => {
    if (!nodes.has(id)) nodes.set(id, { id, label, role, col: 0, row: 0 });
  };

  for (const t of pipeline.teams) addNode(t.id, t.name || t.id, teamRole(t.name || t.id));
  for (const g of pipeline.gates) addNode(g.id, g.label || g.id, "gate");
  for (const f of pipeline.forks) addNode(f.id, f.id, "fork");
  for (const j of pipeline.joins) addNode(j.id, j.id, "join");
  for (const e of pipeline.escalations) addNode(e.id, e.id, "escalation");

  const edges: GraphEdge[] = [];
  const adj = new Map<string, string[]>(); // forward adjacency for ranking
  const pushEdge = (from: string, to: string, kind: EdgeKind) => {
    if (!nodes.has(from) || !nodes.has(to)) return;
    edges.push({ from, to, kind });
    if (kind === "forward") {
      adj.set(from, [...(adj.get(from) ?? []), to]);
    }
  };

  for (const t of pipeline.teams) {
    const o = t.outputs ?? {};
    if (o.on_approve) pushEdge(t.id, o.on_approve, "forward");
    // Revise routes back upstream (a back-edge); escalate goes to a sink.
    if (o.on_revise) pushEdge(t.id, o.on_revise, "revise");
    if (o.on_reject) pushEdge(t.id, o.on_reject, "escalate");
  }
  for (const g of pipeline.gates) {
    if (g.downstream) pushEdge(g.id, g.downstream, "forward");
  }
  for (const f of pipeline.forks) {
    for (const lane of f.lanes) pushEdge(f.id, lane, "forward");
  }
  for (const j of pipeline.joins) {
    for (const w of j.waits_for) pushEdge(w, j.id, "forward");
    if (j.downstream) pushEdge(j.id, j.downstream, "forward");
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
