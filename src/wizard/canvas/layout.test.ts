import { describe, expect, it } from "vitest";
import { layout, type LayoutNode, type LayoutEdge } from "./layout";

const N = (id: string): LayoutNode => ({ id });

describe("canvas layout", () => {
  it("positions every node and is deterministic", () => {
    const nodes = [N("a"), N("b"), N("c")];
    const edges: LayoutEdge[] = [
      { source: "a", target: "b" },
      { source: "b", target: "c" },
    ];
    const p1 = layout(nodes, edges);
    const p2 = layout(nodes, edges);
    expect(Object.keys(p1).sort()).toEqual(["a", "b", "c"]);
    expect(p1).toEqual(p2);
    for (const id of ["a", "b", "c"]) {
      expect(Number.isFinite(p1[id].x)).toBe(true);
      expect(Number.isFinite(p1[id].y)).toBe(true);
    }
  });

  it("ranks a chain left-to-right (downstream node is further right)", () => {
    const nodes = [N("a"), N("b"), N("c")];
    const edges: LayoutEdge[] = [
      { source: "a", target: "b" },
      { source: "b", target: "c" },
    ];
    const p = layout(nodes, edges);
    expect(p.b.x).toBeGreaterThan(p.a.x);
    expect(p.c.x).toBeGreaterThan(p.b.x);
  });

  it("handles a fork/join + revise loop without crashing and places all nodes", () => {
    const nodes = ["fork-1", "a", "b", "join-1", "rev", "writer", "needs-human"].map(N);
    const edges: LayoutEdge[] = [
      { source: "fork-1", target: "a" },
      { source: "fork-1", target: "b" },
      { source: "a", target: "join-1" },
      { source: "b", target: "join-1" },
      { source: "join-1", target: "rev" },
      { source: "rev", target: "writer" }, // revise loop back
      { source: "rev", target: "needs-human" },
    ];
    const p = layout(nodes, edges);
    expect(Object.keys(p).length).toBe(7);
    // the two fork lanes share a rank (same x), distinct rows (different y)
    expect(p.a.x).toBe(p.b.x);
    expect(p.a.y).not.toBe(p.b.y);
  });

  it("positions an isolated node (no edges)", () => {
    const p = layout([N("lonely")], []);
    expect(Number.isFinite(p.lonely.x)).toBe(true);
    expect(Number.isFinite(p.lonely.y)).toBe(true);
  });
});
