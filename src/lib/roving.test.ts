import { describe, expect, it } from "vitest";
import { rovingTabKey } from "./roving";

describe("rovingTabKey", () => {
  it("ArrowRight advances and wraps", () => {
    expect(rovingTabKey("ArrowRight", 0, 5)).toBe(1);
    expect(rovingTabKey("ArrowRight", 4, 5)).toBe(0); // wrap
  });
  it("ArrowLeft retreats and wraps", () => {
    expect(rovingTabKey("ArrowLeft", 2, 5)).toBe(1);
    expect(rovingTabKey("ArrowLeft", 0, 5)).toBe(4); // wrap
  });
  it("Home/End jump to ends", () => {
    expect(rovingTabKey("Home", 3, 5)).toBe(0);
    expect(rovingTabKey("End", 1, 5)).toBe(4);
  });
  it("returns null for keys it does not handle", () => {
    expect(rovingTabKey("Enter", 0, 5)).toBeNull();
    expect(rovingTabKey("a", 0, 5)).toBeNull();
    expect(rovingTabKey(" ", 0, 5)).toBeNull();
  });
  it("returns null for an empty tablist", () => {
    expect(rovingTabKey("ArrowRight", 0, 0)).toBeNull();
  });
});
