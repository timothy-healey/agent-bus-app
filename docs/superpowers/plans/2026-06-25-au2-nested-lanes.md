# AU2 — Nested-Lane (P1) Hierarchical Render Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render a fork nested inside another fork's lane distinctly/hierarchically in the read-only static `PipelineGraph`, so a fork-within-a-lane reads as nested (indented + a containment box + a depth label), honouring the depth-≤3 bound — pure layout, no drag, no backend change.

**Architecture:** All graph reasoning stays in the pure `src/lib/pipelineGraph.ts` (unit-tested with vitest); the SVG renderer `src/components/PipelineGraph.tsx` stays dumb and only draws what the model says. We add a `depth` field per `GraphNode` and a derived list of *nesting groups* (a nested fork + the lane span it owns) by replicating the same forward lane-walk the Rust validator (`src-tauri/pipeline/src/validate.rs::check_lane_reachable`) already performs — walking `fork.lanes` → team `on_approve` hops → gate `downstream` hops → a nested `fork` hop (depth+1) until its paired join. The renderer indents nodes by `depth` and draws a labelled containment `<rect>` behind each nesting group.

**Tech Stack:** TypeScript, React 18, vitest + @testing-library/react (jsdom), SVG. Design tokens from `src/styles/tokens.css`.

---

## Nesting representation — CONFIRMED (read this before Task 1)

The authoring/frontend `Pipeline` model (`src/ipc/pipeline.ts`) has **NO explicit nesting field**. This was verified against the model and the Rust validator:

- `Fork` is `{ id: string; lanes: string[] }`. Each `lanes` entry is a **node id** (the lane's entry node).
- A *nested fork* is expressed **structurally**: a fork is "nested" when it is reachable from inside another fork's lane. Reachability follows the same walk the backend validator does (`src-tauri/pipeline/src/validate.rs:58-128`):
  - a **team** hop follows `team.outputs.on_approve`;
  - a **gate** hop follows `gate.downstream`;
  - a **fork** hop recurses one level deeper (`depth + 1`), and after the nested fork's paired join, continues from that join's `downstream`.
- **Depth** (matching `MAX_NESTING_DEPTH = 3`): a top-level fork is depth 1; a fork reached from inside another fork's lane is depth 2; etc. (`validate.rs:54-56`).
- The nested fork's **paired join** is **derived, not stored**: it is the join all of the nested fork's lanes can reach (`validate.rs:101-103`). We replicate that.

**Conclusion:** Nesting is fully representable from the data the frontend already receives (`forks`, `joins`, `teams[].outputs.on_approve`, `gates[].downstream`). **No backend change is required**, and none is in scope. The frontend graph only ever sees *valid* pipelines (`pipeline_load`/`pipeline_to_draft_cmd` resolve+validate backend-side), so the walk can assume validator invariants hold but must still terminate safely (bounded passes) on any malformed input.

**Gap flag (in-scope honesty):** the model cannot distinguish a fork that is *authored* as nested from one that merely *happens* to be downstream of another fork's lane — there is only the structural walk. We define "nested" exactly as the validator does (reachable inside a lane before that lane's join). That is the only thing the model expresses, and it is what we render.

---

## File Structure

- **Modify `src/lib/pipelineGraph.ts`** — add `depth: number` to `GraphNode`; add exported `forkNestingDepths(pipeline): Map<string, number>` (fork id → depth, 1-based) and `nestingGroups(pipeline): NestingGroup[]` (each nested fork with its member node ids + depth); set `node.depth` during layout. Pure, unit-tested.
- **Modify `src/components/PipelineGraph.tsx`** — indent node x-position by `depth`, draw a labelled containment `<rect>` behind each nesting group, tag groups with `data-nesting-depth` for tests. Dumb renderer.
- **Test `src/lib/pipelineGraph.test.ts`** — add cases for depth + nesting groups.
- **Test `src/components/PipelineGraph` rendering** — assert via existing `src/components/PipelineView.test.tsx` (PipelineGraph has no own test file; PipelineView renders it) that a nested fork renders distinctly.

No new files. No drag. No backend.

---

## Task 1: Add `depth` to GraphNode and a `forkNestingDepths` helper

**Files:**
- Modify: `src/lib/pipelineGraph.ts` (the `GraphNode` interface ~lines 20-26; add helper near `inferTeamRole`, ~line 59)
- Test: `src/lib/pipelineGraph.test.ts`

- [ ] **Step 1: Write the failing test**

Append to `src/lib/pipelineGraph.test.ts` (inside the existing top-level scope, after the `buildPipelineGraph` describe block). The helper `team()` builder mirrors the existing one in the file:

```typescript
import { buildPipelineGraph, nodeBorderColor, inferTeamRole, forkNestingDepths, nestingGroups } from "./pipelineGraph";

// A team builder local to these tests (same shape as the existing one above).
function tm(id: string, on_approve?: string): Pipeline["teams"][number] {
  return {
    id, name: id, prompt: "", scope: { reads: [], writes: [], tools: [] },
    outputs: on_approve ? { on_approve } : {}, workers: { min: 1, max: 1 },
    role: "producer", store: { capacity: 8 },
  };
}

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
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/lib/pipelineGraph.test.ts -t forkNestingDepths`
Expected: FAIL — `forkNestingDepths is not a function` (and `nestingGroups` import unresolved).

- [ ] **Step 3: Add `depth` to the interface and implement `forkNestingDepths`**

In `src/lib/pipelineGraph.ts`, add `depth` to `GraphNode`:

```typescript
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
```

Add this constant + helper after `inferTeamRole` (~line 59). It replicates `check_lane_reachable`'s hop rules but only to assign each fork a depth:

```typescript
/// Max fork nesting depth, mirroring the backend (validate.rs MAX_NESTING_DEPTH).
/// A top-level fork is depth 1; a fork reached inside another fork's lane is 2; etc.
export const MAX_NESTING_DEPTH = 3;

/// fork id -> nesting depth (1-based). Derived purely from the lane-walk the
/// backend validator performs: a fork's depth is 1 plus the depth of the
/// deepest fork whose lane reaches it. We compute this by walking every fork's
/// lanes (depth d) and recording any fork hop encountered at depth d+1, taking
/// the max. Bounded by node count so it terminates on any (even malformed) input.
export function forkNestingDepths(pipeline: Pipeline): Map<string, number> {
  const teams = new Map(pipeline.teams.map((t) => [t.id, t]));
  const gates = new Map(pipeline.gates.map((g) => [g.id, g]));
  const forks = new Map(pipeline.forks.map((f) => [f.id, f]));
  const joins = pipeline.joins;
  const bound =
    pipeline.teams.length + pipeline.gates.length + pipeline.forks.length + pipeline.joins.length + 1;

  const depth = new Map<string, number>();
  for (const f of pipeline.forks) depth.set(f.id, 0);

  // The join all of a fork's lanes can reach (derived pairing, validate.rs:101).
  const pairedJoinDownstream = (forkId: string): string | undefined => {
    const f = forks.get(forkId);
    if (!f) return undefined;
    const j = joins.find((jn) => f.lanes.every((lane) => laneReaches(lane, jn.id)));
    return j?.downstream;
  };

  // Does walking forward from `entry` reach `target` before leaving the lane?
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
        if (!ds) return false;
        cur = ds;
      } else {
        return false;
      }
    }
    return false;
  }

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
        if (!ds) return;
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/lib/pipelineGraph.test.ts -t forkNestingDepths`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/lib/pipelineGraph.ts src/lib/pipelineGraph.test.ts
git commit -m "feat(graph): derive fork nesting depth from the lane-walk (AU2)"
```

---

## Task 2: Compute `nestingGroups` (a nested fork + its member node ids)

**Files:**
- Modify: `src/lib/pipelineGraph.ts` (add `NestingGroup` type + `nestingGroups` export)
- Test: `src/lib/pipelineGraph.test.ts`

- [ ] **Step 1: Write the failing test**

Append a describe block to `src/lib/pipelineGraph.test.ts` (reuses `tm` and `pipe` from earlier):

```typescript
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/lib/pipelineGraph.test.ts -t nestingGroups`
Expected: FAIL — `nestingGroups is not a function`.

- [ ] **Step 3: Implement `NestingGroup` + `nestingGroups`**

In `src/lib/pipelineGraph.ts`, add after `forkNestingDepths`:

```typescript
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
  const teams = new Map(pipeline.teams.map((t) => [t.id, t]));
  const gates = new Map(pipeline.gates.map((g) => [g.id, g]));
  const forks = new Map(pipeline.forks.map((f) => [f.id, f]));
  const joins = pipeline.joins;
  const bound =
    pipeline.teams.length + pipeline.gates.length + pipeline.forks.length + pipeline.joins.length + 1;

  // Collect every node id reachable inside `forkId`'s lanes, up to its join.
  const collect = (forkId: string): string[] => {
    const f = forks.get(forkId);
    if (!f) return [forkId];
    const pairedJoin = joins.find((jn) =>
      f.lanes.every((lane) => {
        let cur = lane;
        for (let i = 0; i <= bound; i++) {
          if (cur === jn.id) return true;
          if (teams.has(cur)) { const n = teams.get(cur)!.outputs?.on_approve; if (!n) return false; cur = n; }
          else if (gates.has(cur)) cur = gates.get(cur)!.downstream;
          else if (forks.has(cur)) { const dj = joins.find((j2) => forks.get(cur)!.lanes.every((l2) => l2 === j2.id || teams.get(l2)?.outputs?.on_approve === j2.id)); if (!dj) return false; cur = dj.downstream; }
          else return false;
        }
        return false;
      }),
    );
    const members = new Set<string>([forkId]);
    if (pairedJoin) members.add(pairedJoin.id);
    for (const lane of f.lanes) {
      let cur = lane;
      for (let i = 0; i <= bound; i++) {
        members.add(cur);
        if (pairedJoin && cur === pairedJoin.id) break;
        if (teams.has(cur)) { const n = teams.get(cur)!.outputs?.on_approve; if (!n) break; cur = n; }
        else if (gates.has(cur)) cur = gates.get(cur)!.downstream;
        else if (forks.has(cur)) { for (const id of collect(cur)) members.add(id); const dj = joins.find((j2) => forks.get(cur)!.lanes.every((l2) => l2 === j2.id || teams.get(l2)?.outputs?.on_approve === j2.id)); if (!dj) break; cur = dj.downstream; }
        else break;
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/lib/pipelineGraph.test.ts -t nestingGroups`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/lib/pipelineGraph.ts src/lib/pipelineGraph.test.ts
git commit -m "feat(graph): compute nesting groups for nested forks (AU2)"
```

---

## Task 3: Stamp `depth` onto every node in `buildPipelineGraph`

**Files:**
- Modify: `src/lib/pipelineGraph.ts` (`buildPipelineGraph`, the `addNode` call ~line 63 and the row/col assignment ~lines 130-139)
- Test: `src/lib/pipelineGraph.test.ts`

- [ ] **Step 1: Write the failing test**

Append to the `buildPipelineGraph` describe block in `src/lib/pipelineGraph.test.ts`:

```typescript
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/lib/pipelineGraph.test.ts -t "stamps a nesting depth"`
Expected: FAIL — `depth` is `undefined` (or 0) for `inner`/`ia` because nothing sets it yet. (`addNode` initialises `depth: 0`.)

- [ ] **Step 3: Initialise and assign depth**

In `buildPipelineGraph`, update `addNode` to seed depth 0:

```typescript
  const addNode = (id: string, label: string, role: NodeRole) => {
    if (!nodes.has(id)) nodes.set(id, { id, label, role, col: 0, row: 0, depth: 0 });
  };
```

Then, just before the `return { nodes: ... }` at the end of the function (after the row/col loop, ~line 142), assign depth from the helpers:

```typescript
  // Hierarchical depth (AU2): a node inside a nested fork's lane span inherits
  // that fork's depth; the fork node itself carries its own depth. A node in
  // several groups takes the deepest (max), capped at MAX_NESTING_DEPTH.
  const forkDepth = forkNestingDepths(pipeline);
  for (const node of nodes.values()) {
    const fd = forkDepth.get(node.id);
    if (fd !== undefined && fd >= 2) node.depth = Math.max(node.depth, fd);
  }
  for (const grp of nestingGroups(pipeline)) {
    for (const id of grp.memberIds) {
      const n = nodes.get(id);
      if (n) n.depth = Math.max(n.depth, grp.depth);
    }
  }
```

Note: `forkDepth` of a top-level fork is 1, which we intentionally do NOT stamp as a node depth (top-level lanes are not indented); only depth>=2 (genuinely nested) raises a node's depth. The test asserts `depth("outer") === 1` — that comes from the fork-id branch; adjust the fork-node branch to stamp the fork's own depth including 1:

```typescript
  for (const node of nodes.values()) {
    const fd = forkDepth.get(node.id);
    if (fd !== undefined) node.depth = Math.max(node.depth, fd); // fork node carries its own depth (1+)
  }
```

(Keep the `nestingGroups` loop below it for the lane members.)

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/lib/pipelineGraph.test.ts`
Expected: PASS (all, including the new depth test). Confirm `depth("after")` is 0 (after is the outer join's downstream, outside every lane span).

- [ ] **Step 5: Commit**

```bash
git add src/lib/pipelineGraph.ts src/lib/pipelineGraph.test.ts
git commit -m "feat(graph): stamp nesting depth onto graph nodes (AU2)"
```

---

## Task 4: Indent nested nodes + draw labelled containment boxes in the renderer

**Files:**
- Modify: `src/components/PipelineGraph.tsx`
- Test: `src/components/PipelineView.test.tsx` (PipelineGraph is rendered through PipelineView; no separate test file exists)

- [ ] **Step 1: Write the failing test**

Append to `src/components/PipelineView.test.tsx` inside the `describe("PipelineView", ...)` block:

```typescript
it("renders a nested fork distinctly with a depth-labelled containment box (AU2)", () => {
  const tm = (id: string, on_approve?: string): Pipeline["teams"][number] => ({
    id, name: id, prompt: "", scope: { reads: [], writes: [], tools: [] },
    outputs: on_approve ? { on_approve } : {}, workers: { min: 1, max: 1 },
    role: "producer", store: { capacity: 8 },
  });
  const p: Pipeline = {
    id: "p", name: "Nested", description: "", schema_version: 2,
    teams: [tm("ia", "inner-join"), tm("ib", "inner-join"), tm("lb", "outer-join"), tm("after")],
    gates: [], escalations: [],
    forks: [
      { id: "outer", lanes: ["inner", "lb"] },
      { id: "inner", lanes: ["ia", "ib"] },
    ],
    joins: [
      { id: "inner-join", waits_for: ["ia", "ib"], downstream: "outer-join" },
      { id: "outer-join", waits_for: ["inner", "lb"], downstream: "after" },
    ],
  };
  const { container } = render(<PipelineView pipeline={p} />);
  // A containment box is drawn for the nested (depth-2) fork group.
  const box = container.querySelector('[data-nesting-depth="2"]');
  expect(box).not.toBeNull();
  // The nested fork node is indented relative to a non-nested node at the same column.
  const innerNode = container.querySelector('[data-node-id="inner"]');
  expect(innerNode).not.toBeNull();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/components/PipelineView.test.tsx -t "nested fork distinctly"`
Expected: FAIL — no element matches `[data-nesting-depth="2"]` (renderer doesn't draw groups yet) and no `[data-node-id]` attribute exists.

- [ ] **Step 3: Render indentation + containment boxes**

In `src/components/PipelineGraph.tsx`:

(a) Import the group helper and add an indent constant near the other layout constants (~line 13):

```typescript
import { buildPipelineGraph, nodeBorderColor, nestingGroups, type RouteKind } from "../lib/pipelineGraph";

const NEST_INDENT = 18; // px added to x per nesting depth level (AU2)
const NEST_PAD = 8;     // px breathing room around a containment box
```

(b) Compute groups and an indent-aware x for each node. Replace the `colX`/`pos` block (~lines 31-33):

```typescript
  const groups = nestingGroups(pipeline);
  const colX = (c: number) => PAD + c * (NODE_W + COL_GAP);
  const rowY = (r: number) => PAD + r * (NODE_H + ROW_GAP);
  const nodeX = (n: (typeof graph.nodes)[number]) => colX(n.col) + n.depth * NEST_INDENT;
  const pos = new Map(graph.nodes.map((n) => [n.id, { x: nodeX(n), y: rowY(n.row) }]));
```

(c) Widen the SVG to allow for the deepest indent (~line 35):

```typescript
  const maxDepth = graph.nodes.reduce((m, n) => Math.max(m, n.depth), 0);
  const width = PAD * 2 + graph.cols * NODE_W + (graph.cols - 1) * COL_GAP + maxDepth * NEST_INDENT + NEST_PAD * 2;
```

(d) Draw containment boxes BEHIND the edges/nodes. Insert immediately after the opening `<svg ...>` `<defs>...</defs>` block, before `{graph.edges.map(...)}`:

```typescript
        {groups.map((grp) => {
          const pts = grp.memberIds
            .map((id) => pos.get(id))
            .filter((p): p is { x: number; y: number } => !!p);
          if (pts.length === 0) return null;
          const minX = Math.min(...pts.map((p) => p.x)) - NEST_PAD;
          const minY = Math.min(...pts.map((p) => p.y)) - NEST_PAD - 12; // room for label
          const maxX = Math.max(...pts.map((p) => p.x)) + NODE_W + NEST_PAD;
          const maxY = Math.max(...pts.map((p) => p.y)) + NODE_H + NEST_PAD;
          return (
            <g key={`nest-${grp.forkId}`} data-nesting-depth={grp.depth} data-nesting-fork={grp.forkId}>
              <rect
                x={minX}
                y={minY}
                width={maxX - minX}
                height={maxY - minY}
                rx={6}
                fill="var(--surface-2)"
                stroke="var(--border-2)"
                strokeWidth={1}
                strokeDasharray="3 3"
              />
              <text
                x={minX + 8}
                y={minY + 12}
                fill="var(--text-3)"
                style={{ fontFamily: "var(--font-mono)", fontSize: 9, letterSpacing: "0.04em" }}
              >
                {`${grp.forkId} · nested ·${grp.depth}`}
              </text>
            </g>
          );
        })}
```

(e) Tag each node `<g>` with its id so tests (and future tooling) can find it. In the node map (~line 90), add `data-node-id`:

```typescript
            <g key={n.id} data-node-role={n.role} data-node-id={n.id} data-node-depth={n.depth}>
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/components/PipelineView.test.tsx`
Expected: PASS (all, including the new AU2 test and the pre-existing fork/edge tests).

- [ ] **Step 5: Commit**

```bash
git add src/components/PipelineGraph.tsx src/components/PipelineView.test.tsx
git commit -m "feat(ui): render nested fork lanes with indentation + containment box (AU2)"
```

---

## Task 5: Honour the depth-≤3 bound visually (cap indent + label)

**Files:**
- Modify: `src/lib/pipelineGraph.ts` (already caps depth at `MAX_NESTING_DEPTH` in Task 1) — add an explicit test that depth never exceeds 3.
- Test: `src/lib/pipelineGraph.test.ts`

- [ ] **Step 1: Write the failing test (regression guard)**

Append to the `forkNestingDepths` describe block:

```typescript
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
```

- [ ] **Step 2: Run test to verify it passes (already capped in Task 1)**

Run: `npx vitest run src/lib/pipelineGraph.test.ts -t "caps nesting depth"`
Expected: PASS — `walkLane` uses `Math.min(d + 1, MAX_NESTING_DEPTH)`. (If it fails, the cap regressed; fix in `forkNestingDepths`.)

- [ ] **Step 3: (No implementation change needed — cap exists.)**

Confirm the `Math.min(d + 1, MAX_NESTING_DEPTH)` in `walkLane` and the indent in the renderer (`n.depth * NEST_INDENT`) naturally honour the bound because depth is clamped before it ever reaches the renderer.

- [ ] **Step 4: Re-run the full lib suite**

Run: `npx vitest run src/lib/pipelineGraph.test.ts`
Expected: PASS (all).

- [ ] **Step 5: Commit**

```bash
git add src/lib/pipelineGraph.test.ts
git commit -m "test(graph): guard nesting depth cap at MAX_NESTING_DEPTH (AU2)"
```

---

## Task 6: Final verification gates

**Files:** none (verification only)

- [ ] **Step 1: Run the full vitest suite**

Run: `npx vitest run`
Expected: PASS — every test, including the pre-existing `PipelineView.test.tsx` fork/edge/empty-state tests and `pipelineGraph.test.ts` original cases, plus all AU2 additions. No regressions.

- [ ] **Step 2: Typecheck**

Run: `npx tsc --noEmit`
Expected: no errors. (Pay attention to the `GraphNode.depth` addition — every place constructing a `GraphNode` literal must now include `depth`; only `addNode` constructs them, already updated.)

- [ ] **Step 3: Commit (only if Steps 1-2 surfaced fixes)**

```bash
git add -A
git commit -m "chore(au2): verification — vitest + tsc green"
```

---

## Self-Review

**Spec coverage:**
- "Confirm how a nested fork is expressed" → done up-front in the "Nesting representation — CONFIRMED" section (structural lane-walk; no explicit field; no backend change; gap flagged).
- "Render nested fork lanes hierarchically/distinctly (indentation/box/labels)" → Task 4 (indent by depth, dashed containment `<rect>`, `forkId · nested ·N` label).
- "Honouring the depth-≤3 bound" → Task 1 cap + Task 5 regression guard.
- "Pure layout, no drag, no backend" → all logic in `pipelineGraph.ts` (pure) + dumb SVG; no Tauri/IPC/Rust touched.
- "Strict TDD over the pure layout + the renderer shows them distinctly" → Tasks 1-3 unit-test the pure model; Task 4 tests the rendered DOM via PipelineView.
- "Existing design tokens" → uses `--surface-2`, `--border-2`, `--text-3`, `--font-mono` (all in `tokens.css`).
- "Verify gates: `npx vitest run`, `npx tsc --noEmit`" → Task 6.

**Type consistency:** `forkNestingDepths` (Map), `nestingGroups`/`NestingGroup` (`forkId`, `depth`, `memberIds`), `GraphNode.depth`, `MAX_NESTING_DEPTH`, `NEST_INDENT`/`NEST_PAD` are used consistently across tasks. `nodeX(n)` consumes `n.depth`. Test data-attributes (`data-nesting-depth`, `data-node-id`, `data-node-depth`) match the renderer.

**Placeholder scan:** none — every code step is complete.
