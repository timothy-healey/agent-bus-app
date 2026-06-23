import { useState, type CSSProperties } from "react";
import { parseMarkdown } from "../lib/markdown";
import { renderBlock } from "../lib/renderBlock";
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

export function ArtifactView({ markdown, onAddComment }: ArtifactViewProps) {
  const [selection, setSelection] = useState<ActiveSelection | null>(null);

  if (!markdown.trim()) {
    return (
      <div style={{ padding: 22, color: "var(--text-3)", fontSize: "var(--ts-base)", fontStyle: "italic" }}>
        no artifact produced yet.
      </div>
    );
  }

  const blocks = parseMarkdown(markdown);

  const body: CSSProperties = {
    overflowY: "auto",
    padding: "18px 22px",
    fontFamily: "var(--font-reading)",
    fontSize: 13.5,
    lineHeight: 1.6,
    color: "var(--text-2)",
    position: "relative",
    height: "100%",
    boxSizing: "border-box",
  };
  // Cap reading-body line length per DESIGN.md §Typography (75ch), D10.
  const measure: CSSProperties = { maxWidth: "75ch" };

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
      <div style={measure}>{blocks.map(renderBlock)}</div>
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
