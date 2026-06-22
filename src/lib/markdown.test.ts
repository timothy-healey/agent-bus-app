import { describe, expect, it } from "vitest";
import { parseMarkdown } from "./markdown";

describe("parseMarkdown", () => {
  it("parses headings by level", () => {
    const blocks = parseMarkdown("# Title\n## Section\n### Sub");
    expect(blocks).toEqual([
      { type: "heading", level: 1, text: "Title" },
      { type: "heading", level: 2, text: "Section" },
      { type: "heading", level: 3, text: "Sub" },
    ]);
  });

  it("parses a paragraph", () => {
    const blocks = parseMarkdown("Hello world.");
    expect(blocks).toEqual([{ type: "paragraph", text: "Hello world." }]);
  });

  it("parses a fenced code block preserving inner newlines", () => {
    const blocks = parseMarkdown("```\nconst a = 1;\nconst b = 2;\n```");
    expect(blocks).toEqual([
      { type: "code", text: "const a = 1;\nconst b = 2;" },
    ]);
  });

  it("parses an unordered list", () => {
    const blocks = parseMarkdown("- one\n- two");
    expect(blocks).toEqual([{ type: "list", items: ["one", "two"] }]);
  });

  it("treats a leading --- frontmatter fence as a frontmatter block", () => {
    const blocks = parseMarkdown("---\nid: T-1\nversion: 1\n---\n# Title");
    expect(blocks[0]).toEqual({ type: "frontmatter", text: "id: T-1\nversion: 1" });
    expect(blocks[1]).toEqual({ type: "heading", level: 1, text: "Title" });
  });

  it("ignores blank lines between blocks", () => {
    const blocks = parseMarkdown("# A\n\nbody\n\n");
    expect(blocks).toEqual([
      { type: "heading", level: 1, text: "A" },
      { type: "paragraph", text: "body" },
    ]);
  });
});
