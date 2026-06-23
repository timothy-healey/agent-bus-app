import { describe, expect, it } from "vitest";
import { lineDiff } from "./lineDiff";

describe("lineDiff", () => {
  it("reports zero changes for identical bodies", () => {
    const d = lineDiff("a\nb\nc", "a\nb\nc");
    expect(d.added).toBe(0);
    expect(d.removed).toBe(0);
    expect(d.lines.every((l) => l.kind === "same")).toBe(true);
  });

  it("counts added and removed lines", () => {
    const d = lineDiff("a\nb\nc", "a\nx\nc\nd");
    // b removed, x + d added
    expect(d.removed).toBe(1);
    expect(d.added).toBe(2);
  });

  it("classifies each line with its source side", () => {
    const d = lineDiff("keep\nold", "keep\nnew");
    const kinds = d.lines.map((l) => `${l.kind}:${l.text}`);
    expect(kinds).toContain("same:keep");
    expect(kinds).toContain("removed:old");
    expect(kinds).toContain("added:new");
  });

  it("normalises CRLF and treats trailing newline as no extra line", () => {
    const d = lineDiff("a\r\nb\r\n", "a\nb\n");
    expect(d.added).toBe(0);
    expect(d.removed).toBe(0);
  });
});
