import { describe, expect, it } from "vitest";
import { draftToPipeline } from "./draftToPipeline";
import { addTeam, emptyDraft } from "./draft";

describe("draftToPipeline", () => {
  it("maps inline prompt bodies to placeholder prompt paths", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const p = draftToPipeline(d);
    expect(p.teams[0].prompt).toBe("prompts/research.md");
  });

  it("falls back to placeholder id/name when blank and carries graph nodes through", () => {
    const d = {
      ...addTeam(emptyDraft(), "a", "A"),
      id: "",
      name: "",
      gates: [{ id: "gate-1", label: "Review", downstream: "a" }],
      forks: [{ id: "fork-1", lanes: ["a"] }],
      joins: [{ id: "join-1", waits_for: ["a"], downstream: "needs-human" }],
    };
    const p = draftToPipeline(d);
    expect(p.id).toBe("(draft)");
    expect(p.name).toBe("(unnamed)");
    expect(p.gates).toEqual(d.gates);
    expect(p.forks).toEqual(d.forks);
    expect(p.joins).toEqual(d.joins);
  });
});
