import { useState, type CSSProperties } from "react";
import { parseMarkdown, type Block } from "../lib/markdown";
import { SelectionPopover } from "./SelectionPopover";

export interface ArtifactViewProps {
  markdown: string;
  /// Called when the operator adds a comment to a selection.
  onAddComment: (note: string, quote: string, offset: number) => void;
  /// Inline anchor spans to highlight (text + marker number), v1 best-effort.
  anchors?: { text: string; marker: number }[];
}

interface ActiveSelection {
  quote: string;
  offset: number;
}

function renderBlock(b: Block, i: number) {
  const codeFont = "'Berkeley Mono','JetBrains Mono',ui-monospace,monospace";
  switch (b.type) {
    case "frontmatter":
      return (
        <pre
          key={i}
          style={{
            background: "var(--bg-2)",
            border: "1px solid var(--border)",
            borderRadius: "var(--r-sm)",
            padding: "10px 12px",
            fontFamily: codeFont,
            fontSize: 12,
            color: "var(--text-3)",
            marginBottom: 18,
            whiteSpace: "pre-wrap",
          }}
        >
          {b.text}
        </pre>
      );
    case "heading": {
      const sizes = { 1: 19, 2: 16, 3: 14 } as const;
      const Tag = (`h${b.level}` as unknown) as "h1";
      return (
        <Tag
          key={i}
          style={{
            color: "var(--text)",
            fontWeight: 600,
            fontSize: sizes[b.level],
            lineHeight: 1.3,
            margin: b.level === 1 ? "0 0 4px" : "22px 0 8px",
          }}
        >
          {b.text}
        </Tag>
      );
    }
    case "code":
      return (
        <pre
          key={i}
          style={{
            background: "var(--bg-2)",
            border: "1px solid var(--border)",
            borderRadius: "var(--r-sm)",
            padding: "10px 12px",
            fontFamily: codeFont,
            fontSize: 12,
            color: "var(--text-2)",
            overflowX: "auto",
            margin: "8px 0 14px",
          }}
        >
          {b.text}
        </pre>
      );
    case "list":
      return (
        <ul key={i} style={{ margin: "8px 0 12px", paddingLeft: 22 }}>
          {b.items.map((it, j) => (
            <li key={j} style={{ margin: "3px 0" }}>
              {it}
            </li>
          ))}
        </ul>
      );
    case "paragraph":
      return (
        <p key={i} style={{ margin: "0 0 10px" }}>
          {b.text}
        </p>
      );
  }
}

export function ArtifactView({ markdown, onAddComment }: ArtifactViewProps) {
  const [selection, setSelection] = useState<ActiveSelection | null>(null);

  if (!markdown.trim()) {
    return (
      <div style={{ padding: 22, color: "var(--text-4)", fontSize: 12, fontStyle: "italic" }}>
        no artifact produced yet.
      </div>
    );
  }

  const blocks = parseMarkdown(markdown);

  const body: CSSProperties = {
    overflowY: "auto",
    padding: "18px 22px",
    fontFamily: "ui-sans-serif, -apple-system, system-ui, sans-serif",
    fontSize: 13.5,
    lineHeight: 1.6,
    color: "var(--text-2)",
    position: "relative",
    height: "100%",
    boxSizing: "border-box",
  };

  function handleMouseUp() {
    const sel = typeof window !== "undefined" ? window.getSelection?.() : null;
    const text = sel?.toString().trim();
    if (text) {
      const offset = markdown.indexOf(text);
      setSelection({ quote: text, offset: offset < 0 ? 0 : offset });
    }
  }

  return (
    <div data-testid="artifact-body" style={body} onMouseUp={handleMouseUp}>
      {blocks.map(renderBlock)}
      {selection && (
        <div style={{ position: "sticky", bottom: 12, marginTop: 12 }}>
          <SelectionPopover
            quote={selection.quote}
            onAdd={(note) => {
              onAddComment(note, selection.quote, selection.offset);
              setSelection(null);
            }}
            onCancel={() => setSelection(null)}
          />
        </div>
      )}
    </div>
  );
}
