import { describe, it, expect } from "vitest";
import { toggleSelection, toRepoRelative, parentOf } from "./fileTree";

describe("toggleSelection", () => {
  it("single-select replaces the whole selection", () => {
    expect(toggleSelection(["a"], "b", false)).toEqual(["b"]);
    expect(toggleSelection([], "b", false)).toEqual(["b"]);
  });

  it("multi-select adds when absent, removes when present, preserving order", () => {
    expect(toggleSelection(["a"], "b", true)).toEqual(["a", "b"]);
    expect(toggleSelection(["a", "b", "c"], "b", true)).toEqual(["a", "c"]);
  });
});

describe("toRepoRelative", () => {
  it("strips the base prefix", () => {
    expect(toRepoRelative("/repo/src/x.ts", "/repo")).toBe("src/x.ts");
    expect(toRepoRelative("/repo/src/x.ts", "/repo/")).toBe("src/x.ts");
  });
  it("returns '.' for the base itself", () => {
    expect(toRepoRelative("/repo", "/repo")).toBe(".");
  });
  it("returns the absolute path when not under base, or no base", () => {
    expect(toRepoRelative("/other/x", "/repo")).toBe("/other/x");
    expect(toRepoRelative("/repo/x", "")).toBe("/repo/x");
  });
});

describe("parentOf", () => {
  it("returns the parent directory", () => {
    expect(parentOf("/repo/src/x.ts")).toBe("/repo/src");
    expect(parentOf("/repo")).toBe("");
    expect(parentOf("/repo/")).toBe("");
  });
});
