import type { CSSProperties } from "react";
import { Handle, Position, type NodeProps, type NodeTypes } from "@xyflow/react";
import type { FlowNodeData, FlowNodeKind } from "../draftFlow";
import type { NodeKind } from "../mutations";
import { nodeBorderColor } from "../../../lib/pipelineGraph";

/// Custom React-Flow node renderers, one per NodeKind (vet F9 — a new kind slots
/// in as one more entry in `nodeTypes` + the `KIND_CHROME` table, no restructure).
/// Each shows: kind glyph, label, a connectable source/target handle, and a
/// validation-badge slot fed by `data.warnings`. Border colours reuse the
/// DESIGN.md §Pipeline-editor role palette via `nodeBorderColor`.

interface KindChrome {
  glyph: string;
  /// border accent token/colour for this kind (DESIGN.md role palette).
  accent: (data: FlowNodeData) => string;
  /// sub-label under the name.
  sublabel: (data: FlowNodeData) => string;
}

const KIND_CHROME: Record<NodeKind, KindChrome> = {
  team: {
    glyph: "◆",
    accent: (d) => nodeBorderColor(d.role === "reviewer" ? "reviewer" : "writer"),
    sublabel: (d) => (d.role === "reviewer" ? "reviewer" : "producer"),
  },
  gate: { glyph: "⏸", accent: () => nodeBorderColor("gate"), sublabel: () => "human gate" },
  fork: { glyph: "⋔", accent: () => nodeBorderColor("fork"), sublabel: () => "fork" },
  join: { glyph: "⋈", accent: () => nodeBorderColor("join"), sublabel: () => "join" },
  escalation: { glyph: "!", accent: () => nodeBorderColor("escalation"), sublabel: () => "escalation" },
};

const baseStyle = (accent: string, selected: boolean, warn: boolean): CSSProperties => ({
  minWidth: 160,
  maxWidth: 220,
  background: selected ? "var(--accent-2)" : "var(--surface)",
  // selected = the design-system selected treatment (accent border + tint);
  // warn falls back to the warn border only when not selected.
  border: `1px solid ${selected ? "var(--accent-bd)" : warn ? "var(--warn)" : accent}`,
  borderRadius: "var(--r-md)",
  padding: "var(--sp-3) var(--sp-4)",
  fontFamily: "var(--font-mono)",
  boxShadow: selected ? "var(--shadow-needs-you)" : "var(--shadow-card)",
});

const handleStyle: CSSProperties = {
  width: 8,
  height: 8,
  background: "var(--surface-3)",
  border: "1px solid var(--border-2)",
};

/// G1 store node — a compact, visually distinct synthetic node showing the
/// bounded-buffer capacity (`▢▢▢ /cap`). Deliberately quieter than a team: dashed
/// neutral border + recessed surface so it reads as plumbing, not a place where
/// work is authored. Render-only; its drawer edits the owning team's
/// `store.capacity`.
const STORE_CELLS_MAX = 5;

function StoreShell({ data, selected }: { data: FlowNodeData; selected: boolean }) {
  const cap = data.capacity ?? 0;
  // Show up to STORE_CELLS_MAX queue cells; the "/cap" readout carries the exact
  // number when capacity exceeds what we draw (an ellipsis hints at the overflow).
  const drawn = Math.min(cap, STORE_CELLS_MAX);
  const cells = "▢".repeat(Math.max(drawn, 1));
  const overflow = cap > STORE_CELLS_MAX;
  return (
    <div
      style={{
        minWidth: 92,
        maxWidth: 140,
        background: selected ? "var(--accent-2)" : "var(--surface-2)",
        border: `1px dashed ${selected ? "var(--accent-bd)" : "var(--border-2)"}`,
        borderRadius: "var(--r-md)",
        padding: "var(--sp-2) var(--sp-3)",
        fontFamily: "var(--font-mono)",
        boxShadow: selected ? "var(--shadow-needs-you)" : "var(--shadow-card)",
        textAlign: "center",
      }}
      data-node-kind="store"
      aria-label={`input store for ${data.storeTeamId ?? data.label}, capacity ${cap}`}
    >
      <Handle type="target" position={Position.Left} style={handleStyle} />
      <div style={{ display: "flex", alignItems: "baseline", justifyContent: "center", gap: "var(--sp-1)" }}>
        <span aria-hidden style={{ color: "var(--text-3)", fontSize: "var(--ts-base)", letterSpacing: "0.08em" }}>
          {cells}{overflow ? "…" : ""}
        </span>
        <span style={{ color: "var(--text-2)", fontSize: "var(--ts-sm)", fontWeight: 600, fontVariantNumeric: "tabular-nums" }}>/{cap}</span>
      </div>
      <div style={{ color: "var(--text-3)", fontSize: "var(--ts-xs)", marginTop: 2, letterSpacing: "0.04em" }}>store</div>
      <Handle type="source" position={Position.Right} style={handleStyle} />
    </div>
  );
}

function NodeShell({ data, selected, kind }: { data: FlowNodeData; selected: boolean; kind: FlowNodeKind }) {
  if (kind === "store") return <StoreShell data={data} selected={selected} />;
  const chrome = KIND_CHROME[kind];
  const warn = data.warnings.length > 0;
  const accent = chrome.accent(data);
  return (
    <div
      style={baseStyle(accent, !!selected, warn)}
      data-node-kind={kind}
      aria-label={`${kind} ${data.label}`}
    >
      <Handle type="target" position={Position.Left} style={handleStyle} />
      <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)" }}>
        <span aria-hidden style={{ color: accent, fontSize: "var(--ts-md)" }}>{chrome.glyph}</span>
        <span style={{ color: "var(--text)", fontSize: "var(--ts-base)", fontWeight: 500, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {data.label}
        </span>
        {warn && (
          <span
            role="img"
            aria-label={data.warnings.join("; ")}
            title={data.warnings.join("; ")}
            style={{ marginLeft: "auto", color: "var(--warn)", fontSize: "var(--ts-sm)" }}
          >
            ⚠
          </span>
        )}
      </div>
      <div style={{ color: "var(--text-3)", fontSize: "var(--ts-xs)", marginTop: 2 }}>
        {chrome.sublabel(data)}
      </div>
      <Handle type="source" position={Position.Right} style={handleStyle} />
    </div>
  );
}

function makeNode(kind: FlowNodeKind) {
  function CanvasNode({ data, selected }: NodeProps) {
    return <NodeShell data={data as unknown as FlowNodeData} selected={!!selected} kind={kind} />;
  }
  CanvasNode.displayName = `CanvasNode(${kind})`;
  return CanvasNode;
}

/// The NodeKind-driven nodeTypes map handed to <ReactFlow nodeTypes={...}>. The
/// synthetic `store` kind (G1) joins the same map (render-only, not a model kind).
export const nodeTypes: NodeTypes = {
  team: makeNode("team"),
  gate: makeNode("gate"),
  fork: makeNode("fork"),
  join: makeNode("join"),
  escalation: makeNode("escalation"),
  store: makeNode("store"),
};

// Exported for direct render tests.
export { NodeShell };
