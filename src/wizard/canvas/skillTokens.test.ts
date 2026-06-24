import { describe, expect, it } from "vitest";
import {
  applyInsertion,
  findTrigger,
  isRecognized,
  matchEntries,
  scanTokens,
} from "./skillTokens";
import type { SkillEntry } from "../../ipc/skills";

function entry(over: Partial<SkillEntry> = {}): SkillEntry {
  return {
    name: "brainstorming",
    kind: "skill",
    namespace: "superpowers",
    description: "",
    verbs: [],
    source: "global",
    qualified: false,
    ...over,
  };
}

describe("findTrigger", () => {
  it("finds a slash token at the caret", () => {
    expect(findTrigger("write /bra", 10)).toEqual({ slash: 6, query: "bra" });
  });
  it("opens on a bare slash with empty query", () => {
    expect(findTrigger("do /", 4)).toEqual({ slash: 3, query: "" });
  });
  it("opens at the very start of the text", () => {
    expect(findTrigger("/vet", 4)).toEqual({ slash: 0, query: "vet" });
  });
  it("does not trigger inside a path (slash mid-word)", () => {
    expect(findTrigger("src/main", 8)).toBeNull();
  });
  it("does not trigger when caret is before any slash", () => {
    expect(findTrigger("hello", 5)).toBeNull();
  });
  it("closes the token at a space", () => {
    // caret after the space → no active trigger
    expect(findTrigger("/vet ", 5)).toBeNull();
  });
  it("matches the qualified form", () => {
    expect(findTrigger("/superpowers:bra", 16)).toEqual({ slash: 0, query: "superpowers:bra" });
  });
});

describe("matchEntries", () => {
  const entries = [
    entry({ name: "brainstorming", namespace: "superpowers" }),
    entry({ name: "ddd-council", namespace: "ddd-council" }),
    entry({ name: "init-session", namespace: null }),
  ];

  it("returns all sorted by name on an empty query", () => {
    expect(matchEntries(entries, "").map((e) => e.name)).toEqual([
      "brainstorming",
      "ddd-council",
      "init-session",
    ]);
  });
  it("prefix-matches the bare name", () => {
    expect(matchEntries(entries, "ddd").map((e) => e.name)).toEqual(["ddd-council"]);
  });
  it("matches the namespace / qualified form", () => {
    expect(matchEntries(entries, "superpowers").map((e) => e.name)).toContain("brainstorming");
  });
  it("ranks prefix above substring", () => {
    const set = [entry({ name: "session-init" }), entry({ name: "init-session" })];
    expect(matchEntries(set, "init").map((e) => e.name)).toEqual(["init-session", "session-init"]);
  });
});

describe("applyInsertion", () => {
  it("inserts the bare form and a trailing space", () => {
    const text = "use /bra";
    const trig = findTrigger(text, 8)!;
    const result = applyInsertion(text, 8, trig, entry({ name: "brainstorming", qualified: false }));
    expect(result.text).toBe("use /brainstorming ");
    expect(result.caret).toBe(result.text.length);
  });
  it("inserts the qualified form when entry.qualified", () => {
    const text = "/bra";
    const trig = findTrigger(text, 4)!;
    const result = applyInsertion(text, 4, trig, entry({ qualified: true }));
    expect(result.text).toBe("/superpowers:brainstorming ");
  });
  it("appends a verb in the cascade", () => {
    const text = "/ddd";
    const trig = findTrigger(text, 4)!;
    const result = applyInsertion(text, 4, trig, entry({ name: "ddd-council", namespace: null }), "vet");
    expect(result.text).toBe("/ddd-council vet ");
  });
  it("does not double a following space", () => {
    const text = "/bra rest";
    const trig = findTrigger(text, 4)!;
    const result = applyInsertion(text, 4, trig, entry({ name: "brainstorming", qualified: false }));
    expect(result.text).toBe("/brainstorming rest");
  });
});

describe("scanTokens", () => {
  it("finds slash tokens at boundaries", () => {
    const toks = scanTokens("run /vet then /map");
    expect(toks.map((t) => t.text)).toEqual(["/vet", "/map"]);
    expect(toks[0].name).toBe("vet");
  });
  it("ignores a slash inside a path", () => {
    expect(scanTokens("edit src/main.rs")).toEqual([]);
  });
  it("captures the qualified form name", () => {
    expect(scanTokens("/superpowers:brainstorming")[0].name).toBe("superpowers:brainstorming");
  });
});

describe("isRecognized", () => {
  const entries = [entry({ name: "brainstorming", namespace: "superpowers" })];
  it("recognizes a bare name", () => {
    expect(isRecognized("brainstorming", entries)).toBe(true);
  });
  it("recognizes a qualified name", () => {
    expect(isRecognized("superpowers:brainstorming", entries)).toBe(true);
  });
  it("rejects an unknown token (typo / not installed)", () => {
    expect(isRecognized("brainstrom", entries)).toBe(false);
  });
});
