import type React from "react";
import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
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
import { Button } from "../components/ui/Button";

/// PipelineCanvas — the primary Pipeline Authoring surface (spec). Controlled by
/// the same {draft, onChange} contract as the form steps it replaces.
/// DraftPipeline is the single source of truth: structure is DERIVED via
/// draftToFlow; node positions live in LOCAL state (auto-layout new nodes,
/// preserve dragged ones) and are NEVER persisted. All edits go through pure
/// draft mutators.

interface PipelineCanvasProps {
  draft: DraftPipeline;
  onChange: (d: DraftPipeline) => void;
}

const PALETTE_LABEL: Record<NodeKind, string> = {
  team: "Team",
  gate: "Gate",
  fork: "Fork",
  join: "Join",
  escalation: "Escalation",
};

function CanvasInner({ draft, onChange }: PipelineCanvasProps) {
  // LOCAL, ephemeral position map (spec §Positions) — never persisted.
  const [positions, setPositions] = useState<Record<string, XY>>({});
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [issues, setIssues] = useState<string[]>([]);

  // Live best-effort validation (backend stays the authority).
  useEffect(() => {
    let active = true;
    bestEffortValidate(draft).then((i) => { if (active) setIssues(i); }).catch(() => {});
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
      style: e.data.dangling
        ? { stroke: "var(--danger)", strokeDasharray: "4 3" }
        : e.data.kind === "revise"
          ? { stroke: "var(--revise)", strokeDasharray: "4 3" }
          : e.data.kind === "reject"
            ? { stroke: "var(--danger)", strokeDasharray: "4 3" }
            : { stroke: "var(--text-3)" },
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
      </div>

      {issues.length > 0 && (
        <ul role="status" aria-label="validation issues" style={banner}>
          {issues.map((iss, i) => (<li key={i}>• {iss}</li>))}
        </ul>
      )}

      <div style={canvasWrap} data-testid="pipeline-canvas">
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

      <NodeDrawer draft={draft} selectedId={selectedId} onChange={onChange} onClose={() => setSelectedId(null)} />
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
