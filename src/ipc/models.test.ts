import { describe, it, expect } from "vitest";
import { CLAUDE_MODELS, DEFAULT_MODEL, isKnownModel } from "./models";

describe("CLAUDE_MODELS curated list", () => {
  it("covers the opus/sonnet/haiku families with exact IDs", () => {
    const ids = CLAUDE_MODELS.map((m) => m.id);
    expect(ids).toContain("claude-opus-4-8");
    expect(ids).toContain("claude-sonnet-4-6");
    expect(ids).toContain("claude-haiku-4-5");
    const families = new Set(CLAUDE_MODELS.map((m) => m.family));
    expect(families).toEqual(new Set(["opus", "sonnet", "haiku"]));
  });

  it("isKnownModel distinguishes curated IDs from overrides", () => {
    expect(isKnownModel("claude-opus-4-8")).toBe(true);
    expect(isKnownModel("claude-experimental-9")).toBe(false);
    expect(isKnownModel("")).toBe(false);
  });
});

describe("DEFAULT_MODEL", () => {
  it("is Opus 5.5", () => {
    expect(DEFAULT_MODEL).toBe("claude-opus-5-5");
  });

  it("leads the curated list, with Sonnet 5.5 also offered", () => {
    expect(CLAUDE_MODELS[0].id).toBe(DEFAULT_MODEL);
    expect(CLAUDE_MODELS.map((m) => m.id)).toContain("claude-sonnet-5-5");
  });
});
