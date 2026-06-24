import type { DraftPipeline } from "../../ipc/pipeline";
import type { RouteKind } from "../../lib/pipelineGraph";
import { inferTeamRole } from "../../lib/pipelineGraph";
import { addTeam, addGate } from "../draft";

/// The authoring node kinds (vet F9 — palette/drawer/renderers are driven by this
/// set so a new model kind slots in as one entry, not a restructure). `fork` and
/// `join` are two palette faces of the one paired fork+join primitive (W4).
export const NODE_KINDS = ["team", "gate", "fork", "join", "escalation"] as const;
export type NodeKind = (typeof NODE_KINDS)[number];

/// Generate an id of the form `<prefix>-<n>` not already used by any node.
function uniqueId(draft: DraftPipeline, prefix: string): string {
  const used = new Set<string>([
    ...draft.teams.map((t) => t.id),
    ...draft.gates.map((g) => g.id),
    ...draft.forks.map((f) => f.id),
    ...draft.joins.map((j) => j.id),
    ...draft.escalations.map((e) => e.id),
  ]);
  let n = 1;
  while (used.has(`${prefix}-${n}`)) n += 1;
  return `${prefix}-${n}`;
}

/// Add a node of `kind` to the draft. Returns the new draft plus the new node's id
/// (the host selects it + opens the drawer). Fork/join both create the W4 paired
/// fork+join primitive; `newId` is the fork id in that case.
export function addNode(draft: DraftPipeline, kind: NodeKind): { draft: DraftPipeline; newId: string } {
  switch (kind) {
    case "team": {
      const id = uniqueId(draft, "team");
      return { draft: addTeam(draft, id, id), newId: id };
    }
    case "gate": {
      const id = uniqueId(draft, "gate");
      return { draft: addGate(draft, id, id, ""), newId: id };
    }
    case "escalation": {
      const id = uniqueId(draft, "needs-human");
      return { draft: { ...draft, escalations: [...draft.escalations, { id, triggers: [] }] }, newId: id };
    }
    case "fork":
    case "join": {
      // The paired fork+join primitive. addForkJoin requires >= 2 lanes; seed two
      // placeholder lanes so the node exists immediately — lanes are then assigned
      // by drawing edges (which replace the placeholders) or via the drawer.
      const suffix = uniqueForkSuffix(draft);
      const forkId = `fork-${suffix}`;
      const joinId = `join-${suffix}`;
      const next = {
        ...draft,
        forks: [...draft.forks, { id: forkId, lanes: [] as string[] }],
        joins: [...draft.joins, { id: joinId, waits_for: [] as string[], downstream: "" }],
      };
      return { draft: next, newId: forkId };
    }
  }
}

function uniqueForkSuffix(draft: DraftPipeline): number {
  let n = 1;
  const forkIds = new Set(draft.forks.map((f) => f.id));
  const joinIds = new Set(draft.joins.map((j) => j.id));
  while (forkIds.has(`fork-${n}`) || joinIds.has(`join-${n}`)) n += 1;
  return n;
}

type NodeClass = "team" | "gate" | "fork" | "join" | "escalation" | "unknown";

function classify(draft: DraftPipeline, id: string): NodeClass {
  if (draft.teams.some((t) => t.id === id)) return "team";
  if (draft.gates.some((g) => g.id === id)) return "gate";
  if (draft.forks.some((f) => f.id === id)) return "fork";
  if (draft.joins.some((j) => j.id === id)) return "join";
  if (draft.escalations.some((e) => e.id === id)) return "escalation";
  return "unknown";
}

const ROUTE_SLOT: Record<Exclude<RouteKind, "hand-off">, "on_approve" | "on_revise" | "on_reject"> = {
  approve: "on_approve",
  revise: "on_revise",
  reject: "on_reject",
};

function setTeamRouteSlot(
  draft: DraftPipeline,
  teamId: string,
  slot: "on_approve" | "on_revise" | "on_reject",
  target: string | null,
): DraftPipeline {
  return { ...draft, teams: draft.teams.map((t) => (t.id === teamId ? { ...t, outputs: { ...t.outputs, [slot]: target } } : t)) };
}

/// Draw an edge source → target as a Route. Role-aware (vet F3): a producer team
/// sets a single hand-off (`on_approve`); a reviewer team uses `routeKind`
/// (default `approve`) to pick the verdict slot. Gate sets `downstream`; fork adds
/// a lane. No-op for an unknown source.
export function connect(draft: DraftPipeline, source: string, target: string, routeKind?: RouteKind): DraftPipeline {
  const cls = classify(draft, source);
  switch (cls) {
    case "team": {
      const team = draft.teams.find((t) => t.id === source)!;
      if (inferTeamRole(team) === "producer") {
        return setTeamRouteSlot(draft, source, "on_approve", target);
      }
      const kind = routeKind && routeKind !== "hand-off" ? routeKind : "approve";
      return setTeamRouteSlot(draft, source, ROUTE_SLOT[kind], target);
    }
    case "gate":
      return { ...draft, gates: draft.gates.map((g) => (g.id === source ? { ...g, downstream: target } : g)) };
    case "fork":
      return {
        ...draft,
        forks: draft.forks.map((f) =>
          f.id === source && !f.lanes.includes(target) ? { ...f, lanes: [...f.lanes, target] } : f,
        ),
      };
    case "join":
      return { ...draft, joins: draft.joins.map((j) => (j.id === source ? { ...j, downstream: target } : j)) };
    default:
      return draft;
  }
}

/// Re-classify an existing reviewer edge (source → target) into a different
/// verdict slot, clearing whichever slot currently points at the target.
export function setRouteKind(draft: DraftPipeline, source: string, target: string, routeKind: RouteKind): DraftPipeline {
  if (classify(draft, source) !== "team") return draft;
  const cleared = removeEdge(draft, source, target);
  return connect(cleared, source, target, routeKind);
}

/// Clear the edge source → target. For a team, clears whichever route slot points
/// at the target (or the named `routeKind` slot). For gate/join, clears
/// downstream; for fork, drops the lane.
export function removeEdge(draft: DraftPipeline, source: string, target: string, routeKind?: RouteKind): DraftPipeline {
  const cls = classify(draft, source);
  switch (cls) {
    case "team":
      return {
        ...draft,
        teams: draft.teams.map((t) => {
          if (t.id !== source) return t;
          const o = { ...t.outputs };
          if (routeKind && routeKind !== "hand-off") {
            if (o[ROUTE_SLOT[routeKind]] === target) o[ROUTE_SLOT[routeKind]] = null;
          } else {
            for (const slot of ["on_approve", "on_revise", "on_reject"] as const) {
              if (o[slot] === target) o[slot] = null;
            }
          }
          return { ...t, outputs: o };
        }),
      };
    case "gate":
      return { ...draft, gates: draft.gates.map((g) => (g.id === source && g.downstream === target ? { ...g, downstream: "" } : g)) };
    case "join":
      return { ...draft, joins: draft.joins.map((j) => (j.id === source && j.downstream === target ? { ...j, downstream: "" } : j)) };
    case "fork":
      return { ...draft, forks: draft.forks.map((f) => (f.id === source ? { ...f, lanes: f.lanes.filter((l) => l !== target) } : f)) };
    default:
      return draft;
  }
}

/// Remove a node and every reference to it (the UI never leaves a dangling ref).
/// Removing a fork or its paired join removes both halves of the primitive.
export function removeNode(draft: DraftPipeline, id: string): DraftPipeline {
  const cls = classify(draft, id);

  // Fork/join are a paired primitive — drop both halves.
  let forks = draft.forks;
  let joins = draft.joins;
  if (cls === "fork") {
    const pairedJoin = id.replace(/^fork-/, "join-");
    forks = forks.filter((f) => f.id !== id);
    joins = joins.filter((j) => j.id !== pairedJoin);
  } else if (cls === "join") {
    const pairedFork = id.replace(/^join-/, "fork-");
    joins = joins.filter((j) => j.id !== id);
    forks = forks.filter((f) => f.id !== pairedFork);
  }

  return {
    ...draft,
    teams: draft.teams
      .filter((t) => t.id !== id)
      .map((t) => ({
        ...t,
        outputs: {
          on_approve: t.outputs.on_approve === id ? null : t.outputs.on_approve,
          on_revise: t.outputs.on_revise === id ? null : t.outputs.on_revise,
          on_reject: t.outputs.on_reject === id ? null : t.outputs.on_reject,
        },
      })),
    gates: draft.gates
      .filter((g) => g.id !== id)
      .map((g) => (g.downstream === id ? { ...g, downstream: "" } : g)),
    escalations: draft.escalations.filter((e) => e.id !== id),
    forks: forks.map((f) => ({ ...f, lanes: f.lanes.filter((l) => l !== id) })),
    joins: joins.map((j) => ({
      ...j,
      waits_for: j.waits_for.filter((w) => w !== id),
      downstream: j.downstream === id ? "" : j.downstream,
    })),
  };
}
