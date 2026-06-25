import type React from "react";
import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  Controls,
  type Connection,
  type Node as RFNode,
  type Edge as RFEdge,
  type NodeChange,
  type EdgeChange,
  applyNodeChanges,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type { DraftPipeline } from "../ipc/pipeline";
import { bestEffortValidate } from "../ipc/pipeline";
import { draftToFlow, reconcile } from "./canvas/draftFlow";
import type { XY } from "./canvas/layout";
import { addNode, connect, removeNode, removeEdge, NODE_KINDS, type NodeKind } from "./canvas/mutations";
import { nodeTypes } from "./canvas/nodes/CanvasNodes";
import { NodeDrawer } from "./canvas/NodeDrawer";
import { NodeContextMenu } from "./canvas/NodeContextMenu";
import { Button } from "../components/ui/Button";
import type { SkillEntry } from "../ipc/skills";

/// PipelineCanvas — the primary Pipeline Authoring surface (spec). Controlled by
/// the same {draft, onChange} contract as the form steps it replaces.
/// DraftPipeline is the single source of truth: structure is DERIVED via
/// draftToFlow; node positions live in LOCAL state (auto-layout new nodes,
/// preserve dragged ones) and are NEVER persisted. All edits go through pure
/// draft mutators.

interface PipelineCanvasProps {
  draft: DraftPipeline;
  onChange: (d: DraftPipeline) => void;
  /// A4 — the discovered skill catalog for the prompt autocomplete (empty when no
  /// project context, e.g. the new-project wizard before create).
  skills?: SkillEntry[];
  /// A4 — re-scan the catalog ("refresh skills" affordance). Hidden when absent.
  onRefreshSkills?: () => void;
  /// G11 — show the PROMINENT validation banner. The subtle live inline node
  /// badges always render; the loud banner appears only when the host raises this
  /// (i.e. on a blocked Continue/Create attempt). Defaults to false.
  showBanner?: boolean;
  /// G11 — report current best-effort validity to the host so it can gate the
  /// nav-tree steps + Continue + Create at attempt time. `issues` is the raw list.
  onValidityChange?: (valid: boolean, issues: string[]) => void;
}

/// Edge stroke per route kind (DESIGN.md §Pipeline-editor edges): hand-off /
/// approve = solid neutral; revise = dashed revise-purple; reject + any dangling
/// route = dashed danger.
function edgeStyle(kind: string, dangling: boolean): CSSProperties {
  if (dangling) return { stroke: "var(--danger)", strokeDasharray: "4 3" };
  if (kind === "revise") return { stroke: "var(--revise)", strokeDasharray: "4 3" };
  if (kind === "reject") return { stroke: "var(--danger)", strokeDasharray: "4 3" };
  return { stroke: "var(--text-3)" };
}

const PALETTE_LABEL: Record<NodeKind, string> = {
  team: "Team",
  gate: "Gate",
  fork: "Fork",
  join: "Join",
  escalation: "Escalation",
};

function CanvasInner({ draft, onChange, skills = [], onRefreshSkills, showBanner = false, onValidityChange }: PipelineCanvasProps) {
  // LOCAL, ephemeral position map (spec §Positions) — never persisted.
  const [positions, setPositions] = useState<Record<string, XY>>({});
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [issues, setIssues] = useState<string[]>([]);
  // G9 — the open right-click context menu (null when closed).
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);

  // Live best-effort validation (backend stays the authority). The result feeds
  // both the subtle inline badges (always) and — via onValidityChange — the host's
  // attempt-time gating (G11). The loud banner is gated separately by showBanner.
  const onValidityChangeRef = useRef(onValidityChange);
  onValidityChangeRef.current = onValidityChange;
  useEffect(() => {
    let active = true;
    bestEffortValidate(draft)
      .then((i) => {
        if (!active) return;
        setIssues(i);
        onValidityChangeRef.current?.(i.length === 0, i);
      })
      .catch(() => {});
    return () => { active = false; };
  }, [draft]);

  const flow = useMemo(() => draftToFlow(draft), [draft]);

  const nodes: RFNode[] = useMemo(
    () => reconcile(flow.nodes, flow.edges, positions).map((n) => ({
      id: n.id,
      type: n.type,
      position: n.position,
      data: n.data as unknown as Record<string, unknown>,
      selected: n.id === selectedId,
    })),
    [flow, positions, selectedId],
  );

  const edges: RFEdge[] = useMemo(
    () => flow.edges.map((e) => ({
      id: e.id,
      source: e.source,
      target: e.target,
      label: e.label,
      animated: e.data.kind === "revise" || e.data.kind === "reject",
      style: edgeStyle(e.data.kind, e.data.dangling),
    })),
    [flow],
  );

  // Capture drag positions locally; structure changes are owned by the draft.
  // (Selection is handled by onNodeClick/onPaneClick.)
  const onNodesChange = useCallback((changes: NodeChange[]) => {
    if (!changes.some((c) => c.type === "position")) return;
    const applied = applyNodeChanges(changes, nodes);
    setPositions((prev) => {
      const next = { ...prev };
      for (const n of applied) next[n.id] = n.position;
      return next;
    });
  }, [nodes]);

  const onEdgesChange = useCallback((changes: EdgeChange[]) => {
    for (const c of changes) {
      if (c.type === "remove") {
        const e = flow.edges.find((x) => x.id === c.id);
        if (e) onChange(removeEdge(draft, e.source, e.target, e.data.kind));
      }
    }
  }, [flow, draft, onChange]);

  const onNodesDelete = useCallback((deleted: RFNode[]) => {
    let next = draft;
    for (const n of deleted) next = removeNode(next, n.id);
    onChange(next);
    setSelectedId(null);
  }, [draft, onChange]);

  // G9 — delete one node by id through the shared removeNode (clears dangling
  // routes). Used by the context menu, the Delete/Backspace key, and the drawer.
  const deleteNode = useCallback((id: string) => {
    onChange(removeNode(draft, id));
    setSelectedId((cur) => (cur === id ? null : cur));
    setMenu(null);
  }, [draft, onChange]);

  // G9 — right-click a node → open the context menu at the cursor.
  const onNodeContextMenu = useCallback((e: React.MouseEvent, n: RFNode) => {
    e.preventDefault();
    setSelectedId(n.id);
    setMenu({ id: n.id, x: e.clientX, y: e.clientY });
  }, []);

  // G9 — Delete/Backspace removes the selected node, but NOT while the caret is in
  // a text field (so editing a node's name/prompt isn't hijacked). React Flow's own
  // onNodesDelete only fires for its internal key handling; this covers the
  // selected-via-drawer case and keeps the guard explicit.
  const onCanvasKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key !== "Delete" && e.key !== "Backspace") return;
    const el = document.activeElement as HTMLElement | null;
    const tag = el?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el?.isContentEditable) return;
    if (!selectedId) return;
    e.preventDefault();
    deleteNode(selectedId);
  }, [selectedId, deleteNode]);

  // Draw an edge → role-aware Route (producer hand-off vs reviewer approve).
  const onConnect = useCallback((c: Connection) => {
    if (!c.source || !c.target) return;
    onChange(connect(draft, c.source, c.target));
  }, [draft, onChange]);

  const onNodeClick = useCallback((_: React.MouseEvent, n: RFNode) => setSelectedId(n.id), []);
  const onPaneClick = useCallback(() => setSelectedId(null), []);

  const add = useCallback((kind: NodeKind) => {
    const { draft: next, newId } = addNode(draft, kind);
    onChange(next);
    setSelectedId(newId); // select + open drawer
  }, [draft, onChange]);

  return (
    <div style={shell}>
      <div style={paletteBar} role="toolbar" aria-label="add node">
        <span style={{ fontSize: "var(--ts-sm)", color: "var(--text-3)", marginRight: "var(--sp-2)" }}>Add</span>
        {NODE_KINDS.map((k) => (
          <Button key={k} size="sm" onClick={() => add(k)} aria-label={`add ${k}`}>{PALETTE_LABEL[k]}</Button>
        ))}
        {onRefreshSkills && (
          <span style={{ marginLeft: "auto" }}>
            <Button size="sm" variant="ghost" onClick={onRefreshSkills} aria-label="refresh skills" title="Re-scan installed skills + commands">
              Refresh skills
            </Button>
          </span>
        )}
      </div>

      {/* G11 — the PROMINENT banner only appears on a blocked attempt (showBanner);
          the subtle live inline node badges carry validity the rest of the time. */}
      {showBanner && issues.length > 0 && (
        <ul role="alert" aria-label="validation issues" style={banner}>
          {issues.map((iss, i) => (<li key={i}>• {iss}</li>))}
        </ul>
      )}

      <div
        style={canvasWrap}
        data-testid="pipeline-canvas"
        tabIndex={-1}
        onKeyDown={onCanvasKeyDown}
      >
        {flow.nodes.length === 0 && (
          <div style={emptyState} aria-hidden>
            Empty pipeline. Add a Team from the palette, or generate a recommended graph from Basics.
          </div>
        )}
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onNodesDelete={onNodesDelete}
          onNodeContextMenu={onNodeContextMenu}
          onConnect={onConnect}
          onNodeClick={onNodeClick}
          onPaneClick={onPaneClick}
          fitView
          proOptions={{ hideAttribution: true }}
        >
          <Background />
          <Controls />
        </ReactFlow>
      </div>

      {menu && (
        <NodeContextMenu
          x={menu.x}
          y={menu.y}
          nodeId={menu.id}
          actions={[{ id: "delete", label: "Delete node", destructive: true, onSelect: () => deleteNode(menu.id) }]}
          onClose={() => setMenu(null)}
        />
      )}

      <NodeDrawer draft={draft} selectedId={selectedId} onChange={onChange} onClose={() => setSelectedId(null)} skills={skills} onDelete={deleteNode} />
    </div>
  );
}

export function PipelineCanvas(props: PipelineCanvasProps) {
  return (
    <ReactFlowProvider>
      <CanvasInner {...props} />
    </ReactFlowProvider>
  );
}

const shell: CSSProperties = { display: "flex", flexDirection: "column", height: "100%", minHeight: 0 };
const paletteBar: CSSProperties = { display: "flex", alignItems: "center", gap: "var(--sp-2)", padding: "var(--sp-2) 0", flexWrap: "wrap" };
const banner: CSSProperties = { listStyle: "none", margin: "0 0 var(--sp-2)", padding: "var(--sp-2)", border: "1px solid var(--warn)", background: "var(--warn-2)", borderRadius: "var(--r-sm)", color: "var(--text-2)", fontSize: "var(--ts-sm)" };
const canvasWrap: CSSProperties = { position: "relative", flex: 1, minHeight: 320, border: "1px solid var(--border)", borderRadius: "var(--r-md)", overflow: "hidden", background: "var(--bg-2)" };
const emptyState: CSSProperties = { position: "absolute", inset: 0, display: "flex", alignItems: "center", justifyContent: "center", textAlign: "center", padding: "var(--sp-8)", color: "var(--text-3)", fontSize: "var(--ts-base)", pointerEvents: "none", zIndex: 1 };
