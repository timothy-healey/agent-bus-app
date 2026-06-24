import dagre from "dagre";

/// Pure auto-layout for the canvas (spec §Positions). Deterministic: same input →
/// same positions, so re-layout never jitters. dagre handles fork/join fan-out and
/// revise back-edges (which a naive longest-path ranking laid out poorly).

export interface LayoutNode {
  id: string;
  width?: number;
  height?: number;
}

export interface LayoutEdge {
  source: string;
  target: string;
}

export interface XY {
  x: number;
  y: number;
}

export const NODE_W = 180;
export const NODE_H = 64;

/// Compute a position per node id. Left-to-right rank flow (dagre `LR`). Returns
/// top-left coordinates (React Flow's `position` origin), not dagre's centre.
export function layout(nodes: LayoutNode[], edges: LayoutEdge[]): Record<string, XY> {
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "LR", nodesep: 28, ranksep: 90, marginx: 24, marginy: 24 });
  g.setDefaultEdgeLabel(() => ({}));

  const ids = new Set(nodes.map((n) => n.id));
  for (const n of nodes) {
    g.setNode(n.id, { width: n.width ?? NODE_W, height: n.height ?? NODE_H });
  }
  for (const e of edges) {
    // Skip edges that reference a node not in the set (tolerant of partial drafts).
    if (ids.has(e.source) && ids.has(e.target)) g.setEdge(e.source, e.target);
  }

  dagre.layout(g);

  const out: Record<string, XY> = {};
  for (const n of nodes) {
    const node = g.node(n.id) as { x: number; y: number; width: number; height: number } | undefined;
    if (node) {
      // dagre gives centre coords; React Flow wants top-left.
      out[n.id] = { x: Math.round(node.x - node.width / 2), y: Math.round(node.y - node.height / 2) };
    } else {
      out[n.id] = { x: 0, y: 0 };
    }
  }
  return out;
}
