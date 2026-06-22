export type Block =
  | { type: "frontmatter"; text: string }
  | { type: "heading"; level: 1 | 2 | 3; text: string }
  | { type: "paragraph"; text: string }
  | { type: "code"; text: string }
  | { type: "list"; items: string[] };

/// Minimal CSP-safe markdown parser covering the subset agent artifacts use:
/// leading `---` frontmatter, h1-h3, paragraphs, fenced code, unordered lists.
/// Inline `code` spans are left in the text and styled by the renderer.
export function parseMarkdown(src: string): Block[] {
  const lines = src.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let i = 0;

  // Leading frontmatter fence.
  if (lines[0]?.trim() === "---") {
    const fm: string[] = [];
    i = 1;
    while (i < lines.length && lines[i].trim() !== "---") {
      fm.push(lines[i]);
      i++;
    }
    i++; // consume closing ---
    blocks.push({ type: "frontmatter", text: fm.join("\n").trim() });
  }

  while (i < lines.length) {
    const line = lines[i];

    if (line.trim() === "") {
      i++;
      continue;
    }

    // Fenced code.
    if (line.trim().startsWith("```")) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith("```")) {
        code.push(lines[i]);
        i++;
      }
      i++; // consume closing fence
      blocks.push({ type: "code", text: code.join("\n") });
      continue;
    }

    // Heading.
    const h = /^(#{1,3})\s+(.*)$/.exec(line);
    if (h) {
      blocks.push({
        type: "heading",
        level: h[1].length as 1 | 2 | 3,
        text: h[2].trim(),
      });
      i++;
      continue;
    }

    // Unordered list.
    if (/^[-*]\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^[-*]\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^[-*]\s+/, "").trim());
        i++;
      }
      blocks.push({ type: "list", items });
      continue;
    }

    // Paragraph (single line; consecutive non-blank lines join with a space).
    const para: string[] = [line.trim()];
    i++;
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !/^(#{1,3})\s+/.test(lines[i]) &&
      !lines[i].trim().startsWith("```") &&
      !/^[-*]\s+/.test(lines[i])
    ) {
      para.push(lines[i].trim());
      i++;
    }
    blocks.push({ type: "paragraph", text: para.join(" ") });
  }

  return blocks;
}
