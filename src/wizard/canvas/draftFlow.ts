import type { DraftPipeline, DraftTeam } from "../../ipc/pipeline";
import type { RouteKind } from "../../lib/pipelineGraph";
import { inferTeamRole } from "../../lib/pipelineGraph";
import type { NodeKind } from "./mutations";
import { layout, type XY } from "./layout";

/// Pure adapter: DraftPipeline → React-Flow nodes + edges (spec §draftToFlow).
/// TOLERANT of partial/invalid drafts — missing prompts and dangling routes
/// become node/edge `data` flags, never a throw (the canvas renders mid-build
/// drafts gracefully). Positions are NOT set here; `reconcile` assigns them so the
/// structure stays derived from the draft while positions stay local.

export interface FlowNodeData {
  kind: NodeKind;
  label: string;
  /// behavioural role for teams (producer/reviewer); undefined for other kinds.
  role?: "producer" | "reviewer";
  /// best-effort, derived-from-draft warnings (e.g. "no prompt yet").
  warnings: string[];
}

export interface FlowNode {
  id: string;
  type: NodeKind;
  position: XY;
  data: FlowNodeData;
}

export interface FlowEdgeData {
  kind: RouteKind;
  /// true when the target node id does not exist in the draft (dangling route).
  dangling: boolean;
}

export interface FlowEdge {
  id: string;
  source: string;
  target: string;
  label: string;
  data: FlowEdgeData;
}

export interface Flow {
  nodes: FlowNode[];
  edges: FlowEdge[];
}

const ZERO: XY = { x: 0, y: 0 };

function teamWarnings(t: DraftTeam): string[] {
  const w: string[] = [];
  if (!t.prompt_body || t.prompt_body.trim() === "") w.push("no prompt yet");
  return w;
}

export function draftToFlow(draft: DraftPipeline): Flow {
  const nodes: FlowNode[] = [];
  const known = new Set<string>();
  const push = (id: string, type: NodeKind, label: string, extra: Partial<FlowNodeData> = {}) => {
    nodes.push({ id, type, position: ZERO, data: { kind: type, label, warnings: [], ...extra } });
    known.add(id);
  };

  for (const t of draft.teams) {
    push(t.id, "team", t.name || t.id, { role: inferTeamRole(t), warnings: teamWarnings(t) });
  }
  for (const g of draft.gates) push(g.id, "gate", g.label || g.id);
  for (const f of draft.forks) push(f.id, "fork", f.id);
  for (const j of draft.joins) push(j.id, "join", j.id);
  for (const e of draft.escalations) push(e.id, "escalation", e.id);

  const edges: FlowEdge[] = [];
  const addEdge = (source: string, target: string | null | undefined, kind: RouteKind) => {
    if (!target) return;
    edges.push({
      id: `${source}::${kind}::${target}`,
      source,
      target,
      label: kind,
      data: { kind, dangling: !known.has(target) },
    });
  };

  for (const t of draft.teams) {
    const handoff = inferTeamRole(t) === "producer";
    const o = t.outputs ?? {};
    addEdge(t.id, o.on_approve, handoff ? "hand-off" : "approve");
    addEdge(t.id, o.on_revise, "revise");
    addEdge(t.id, o.on_reject, "reject");
  }
  for (const g of draft.gates) addEdge(g.id, g.downstream, "approve");
  for (const f of draft.forks) for (const lane of f.lanes) addEdge(f.id, lane, "hand-off");
  for (const j of draft.joins) {
    for (const w of j.waits_for) addEdge(w, j.id, "hand-off");
    addEdge(j.id, j.downstream, "hand-off");
  }

  return { nodes, edges };
}

/// Merge the derived structure with the LOCAL position map: nodes already in
/// `positions` keep their (possibly dragged) coordinates; nodes that are new get
/// auto-laid-out together with the full graph so they land sensibly. Positions are
/// never persisted (spec §Positions).
export function reconcile(nodes: FlowNode[], edges: FlowEdge[], positions: Record<string, XY>): FlowNode[] {
  const hasNew = nodes.some((n) => !(n.id in positions));
  const auto = hasNew ? layout(nodes.map((n) => ({ id: n.id })), edges.map((e) => ({ source: e.source, target: e.target }))) : {};
  return nodes.map((n) => ({
    ...n,
    position: positions[n.id] ?? auto[n.id] ?? ZERO,
  }));
}
