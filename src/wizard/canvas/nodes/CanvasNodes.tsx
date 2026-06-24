import type { CSSProperties } from "react";
import { Handle, Position, type NodeProps, type NodeTypes } from "@xyflow/react";
import type { FlowNodeData } from "../draftFlow";
import type { NodeKind } from "../mutations";

/// Custom React-Flow node renderers, one per NodeKind (vet F9 — a new kind slots
/// in as one more entry in `nodeTypes` + the `KIND_CHROME` table, no restructure).
/// Each shows: kind glyph, label, a connectable source/target handle, and a
/// validation-badge slot fed by `data.warnings`.

interface KindChrome {
  glyph: string;
  /// border accent token for this kind.
  accent: string;
  /// sub-label under the name.
  sublabel: (data: FlowNodeData) => string;
}

const KIND_CHROME: Record<NodeKind, KindChrome> = {
  team: {
    glyph: "◆",
    accent: "var(--accent-bd)",
    sublabel: (d) => (d.role === "reviewer" ? "reviewer" : "producer"),
  },
  gate: { glyph: "⏸", accent: "var(--accent-bd)", sublabel: () => "human gate" },
  fork: { glyph: "⋔", accent: "var(--border-2)", sublabel: () => "fork" },
  join: { glyph: "⋈", accent: "var(--border-2)", sublabel: () => "join" },
  escalation: { glyph: "!", accent: "var(--danger)", sublabel: () => "escalation" },
};

const baseStyle = (accent: string, selected: boolean, warn: boolean): CSSProperties => ({
  minWidth: 160,
  maxWidth: 220,
  background: "var(--surface)",
  border: `1px solid ${warn ? "var(--warn)" : accent}`,
  borderRadius: "var(--r-md)",
  padding: "var(--sp-3) var(--sp-4)",
  fontFamily: "var(--font-mono)",
  boxShadow: selected ? "0 0 0 2px var(--accent)" : "var(--shadow-card)",
  outline: "none",
});

const handleStyle: CSSProperties = {
  width: 9,
  height: 9,
  background: "var(--surface-3)",
  border: "1px solid var(--border-2)",
};

function NodeShell({ data, selected, kind }: { data: FlowNodeData; selected: boolean; kind: NodeKind }) {
  const chrome = KIND_CHROME[kind];
  const warn = data.warnings.length > 0;
  return (
    <div
      style={baseStyle(chrome.accent, !!selected, warn)}
      data-node-kind={kind}
      aria-label={`${kind} ${data.label}`}
    >
      <Handle type="target" position={Position.Left} style={handleStyle} />
      <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)" }}>
        <span aria-hidden style={{ color: chrome.accent, fontSize: "var(--ts-md)" }}>{chrome.glyph}</span>
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

function makeNode(kind: NodeKind) {
  function CanvasNode({ data, selected }: NodeProps) {
    return <NodeShell data={data as unknown as FlowNodeData} selected={!!selected} kind={kind} />;
  }
  CanvasNode.displayName = `CanvasNode(${kind})`;
  return CanvasNode;
}

/// The NodeKind-driven nodeTypes map handed to <ReactFlow nodeTypes={...}>.
export const nodeTypes: NodeTypes = {
  team: makeNode("team"),
  gate: makeNode("gate"),
  fork: makeNode("fork"),
  join: makeNode("join"),
  escalation: makeNode("escalation"),
};

// Exported for direct render tests.
export { NodeShell };
