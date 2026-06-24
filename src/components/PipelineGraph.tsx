import type { CSSProperties } from "react";
import type { Pipeline } from "../ipc/pipeline";
import { buildPipelineGraph, nodeBorderColor, type RouteKind } from "../lib/pipelineGraph";

/// Read-only static graph render of the pipeline (audit Decision 2). Nodes +
/// edges laid out left-to-right, honouring fork/join/gate and revise/escalate
/// back-edges. Static: no drag, no editing. The form below still does the edits.

const NODE_W = 150;
const NODE_H = 40;
const COL_GAP = 70;
const ROW_GAP = 22;
const PAD = 16;

function edgeStroke(kind: RouteKind): CSSProperties {
  switch (kind) {
    case "hand-off":
    case "approve":
      return { stroke: "oklch(40% 0.008 60)", strokeWidth: 1.5 };
    case "revise":
      return { stroke: "var(--revise)", strokeWidth: 1.5, strokeDasharray: "4 3" };
    case "reject":
      return { stroke: "var(--danger)", strokeWidth: 1.5, strokeDasharray: "4 3" };
  }
}

export function PipelineGraph({ pipeline }: { pipeline: Pipeline }) {
  const graph = buildPipelineGraph(pipeline);
  if (graph.nodes.length === 0) return null;

  const colX = (c: number) => PAD + c * (NODE_W + COL_GAP);
  const rowY = (r: number) => PAD + r * (NODE_H + ROW_GAP);
  const pos = new Map(graph.nodes.map((n) => [n.id, { x: colX(n.col), y: rowY(n.row) }]));

  const width = PAD * 2 + graph.cols * NODE_W + (graph.cols - 1) * COL_GAP;
  const height = PAD * 2 + (graph.rows + 1) * NODE_H + graph.rows * ROW_GAP;

  return (
    <div
      data-testid="pipeline-graph"
      role="img"
      aria-label="pipeline flow graph"
      style={{
        position: "relative",
        overflowX: "auto",
        border: "1px solid var(--border)",
        borderRadius: "var(--r-md)",
        background:
          "var(--bg-2) radial-gradient(circle at 1px 1px, var(--border) 1px, transparent 0)",
        backgroundSize: "16px 16px",
        padding: 0,
        marginBottom: "var(--sp-4)",
      }}
    >
      <svg width={width} height={height} style={{ display: "block", minWidth: "100%" }}>
        <defs>
          <marker id="abp-arrow" markerWidth="7" markerHeight="7" refX="6" refY="3" orient="auto">
            <path d="M0,0 L6,3 L0,6 Z" fill="oklch(45% 0.008 60)" />
          </marker>
        </defs>
        {graph.edges.map((e, i) => {
          const a = pos.get(e.from);
          const b = pos.get(e.to);
          if (!a || !b) return null;
          const x1 = a.x + NODE_W;
          const y1 = a.y + NODE_H / 2;
          const x2 = b.x;
          const y2 = b.y + NODE_H / 2;
          const stroke = edgeStroke(e.kind);
          // Back-edges (revise/reject, or any right-to-left link) bow with a
          // Bezier so they read as returns, not forward flow.
          const backward = e.kind === "revise" || e.kind === "reject" || b.x <= a.x;
          const d = backward
            ? `M ${x1} ${y1} C ${x1 + 40} ${y1 - 36}, ${x2 - 40} ${y2 - 36}, ${x2} ${y2}`
            : `M ${x1} ${y1} C ${x1 + COL_GAP / 2} ${y1}, ${x2 - COL_GAP / 2} ${y2}, ${x2} ${y2}`;
          return (
            <path
              key={i}
              d={d}
              fill="none"
              markerEnd="url(#abp-arrow)"
              data-edge-kind={e.kind}
              style={stroke}
            />
          );
        })}
        {graph.nodes.map((n) => {
          const p = pos.get(n.id)!;
          return (
            <g key={n.id} data-node-role={n.role}>
              <rect
                x={p.x}
                y={p.y}
                width={NODE_W}
                height={NODE_H}
                rx={4}
                fill="var(--surface)"
                stroke={nodeBorderColor(n.role)}
                strokeWidth={1.5}
              />
              <text
                x={p.x + 10}
                y={p.y + 16}
                fill="var(--text)"
                style={{ fontFamily: "var(--font-mono)", fontSize: 11 }}
              >
                {n.label.length > 18 ? `${n.label.slice(0, 17)}…` : n.label}
              </text>
              <text
                x={p.x + 10}
                y={p.y + 30}
                fill="var(--text-3)"
                style={{ fontFamily: "var(--font-mono)", fontSize: 9 }}
              >
                {n.role}
              </text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}
