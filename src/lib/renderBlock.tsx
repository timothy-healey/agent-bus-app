import type { Block } from "./markdown";

/// Shared CSP-safe block renderer for agent-artifact markdown. Used by the
/// single-pane ArtifactView and the two-column CompareView (B2) — both Review
/// surfaces; the sharing stays within the Review context (not a cross-context
/// kernel). Pure render — no comment / selection behaviour (that stays
/// ArtifactView-only).
export function renderBlock(b: Block, i: number) {
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
