import { describe, expect, it } from "vitest";
import { emptyDraft, WIZARD_STEPS, renameTeam, setPromptBody, setTeamModel, addTeam, removeTeam } from "./draft";
import { setTeamEffort, setTeamTools, setTeamReads, setTeamWrites } from "./draft";
import { addGate, removeGate, setTeamApprove } from "./draft";
import { addForkJoin, removeForkJoin } from "./draft";

describe("wizard draft helpers", () => {
  it("emptyDraft has no teams + current schema version", () => {
    const d = emptyDraft();
    expect(d.teams).toEqual([]);
    expect(d.schema_version).toBeGreaterThanOrEqual(2);
  });

  it("emptyDraft seeds an empty gates array", () => {
    expect(emptyDraft().gates).toEqual([]);
  });

  it("WIZARD_STEPS lists the five steps in order", () => {
    expect(WIZARD_STEPS).toEqual(["basics", "teams", "prompts", "wiring", "review"]);
  });

  it("addTeam appends a defaulted team", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    expect(d.teams).toHaveLength(1);
    expect(d.teams[0].id).toBe("research");
    expect(d.teams[0].runner.kind).toBe("claude-cli");
  });

  it("renameTeam changes the display name only", () => {
    const d = renameTeam(addTeam(emptyDraft(), "research", "Research"), "research", "Investigators");
    expect(d.teams[0].name).toBe("Investigators");
    expect(d.teams[0].id).toBe("research");
  });

  it("setPromptBody sets one team's body", () => {
    const d = setPromptBody(addTeam(emptyDraft(), "research", "Research"), "research", "investigate the repo");
    expect(d.teams[0].prompt_body).toBe("investigate the repo");
  });

  it("setTeamModel edits the advanced config", () => {
    const d = setTeamModel(addTeam(emptyDraft(), "research", "Research"), "research", "claude-haiku-4");
    expect(d.teams[0].runner.model).toBe("claude-haiku-4");
  });

  it("removeTeam drops the team", () => {
    const d = removeTeam(addTeam(emptyDraft(), "research", "Research"), "research");
    expect(d.teams).toHaveLength(0);
  });
});

describe("advanced team config helpers (W2)", () => {
  const base = addTeam(emptyDraft(), "research", "Research");

  it("setTeamEffort sets a preset EffortMode", () => {
    const d = setTeamEffort(base, "research", { mode: "extended-high" });
    expect(d.teams[0].runner.effort).toEqual({ mode: "extended-high" });
  });

  it("setTeamEffort sets a custom EffortMode with a budget", () => {
    const d = setTeamEffort(base, "research", { mode: "custom", budget_tokens: 16000 });
    expect(d.teams[0].runner.effort).toEqual({ mode: "custom", budget_tokens: 16000 });
  });

  it("setTeamTools parses a comma list into a trimmed string array", () => {
    const d = setTeamTools(base, "research", "Read, Grep ,  Bash ");
    expect(d.teams[0].scope.tools).toEqual(["Read", "Grep", "Bash"]);
  });

  it("setTeamTools drops empty entries", () => {
    const d = setTeamTools(base, "research", "Read,,");
    expect(d.teams[0].scope.tools).toEqual(["Read"]);
  });

  it("setTeamReads / setTeamWrites set scope.reads / scope.writes", () => {
    const d1 = setTeamReads(base, "research", "src/**, docs/**");
    expect(d1.teams[0].scope.reads).toEqual(["src/**", "docs/**"]);
    const d2 = setTeamWrites(base, "research", "artifacts/**");
    expect(d2.teams[0].scope.writes).toEqual(["artifacts/**"]);
  });
});

describe("gate helpers (W3)", () => {
  const base = addTeam(addTeam(emptyDraft(), "plan-writers", "Plan Writers"), "implementers", "Implementers");

  it("addGate appends a gate node", () => {
    const d = addGate(base, "gate-2", "Plan review", "implementers");
    expect(d.gates).toEqual([{ id: "gate-2", label: "Plan review", downstream: "implementers" }]);
  });

  it("addGate is a no-op on a duplicate id", () => {
    const d = addGate(addGate(base, "gate-2", "Plan review", "implementers"), "gate-2", "again", "implementers");
    expect(d.gates).toHaveLength(1);
  });

  it("setTeamApprove repoints a team's on_approve", () => {
    const d = setTeamApprove(base, "plan-writers", "gate-2");
    expect(d.teams.find((t) => t.id === "plan-writers")?.outputs.on_approve).toBe("gate-2");
  });

  it("removeGate drops the gate node", () => {
    const d = removeGate(addGate(base, "gate-2", "Plan review", "implementers"), "gate-2");
    expect(d.gates).toHaveLength(0);
  });
});

describe("fork/join helpers (W4)", () => {
  const base = addTeam(addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B"), "c", "C");

  it("addForkJoin creates a paired fork and join", () => {
    const d = addForkJoin(base, "fork-1", "join-1", ["a", "b"], "c");
    expect(d.forks).toEqual([{ id: "fork-1", lanes: ["a", "b"] }]);
    expect(d.joins).toEqual([{ id: "join-1", waits_for: ["a", "b"], downstream: "c" }]);
  });

  it("addForkJoin is a no-op with fewer than 2 lanes", () => {
    const d = addForkJoin(base, "fork-1", "join-1", ["a"], "c");
    expect(d.forks).toHaveLength(0);
    expect(d.joins).toHaveLength(0);
  });

  it("addForkJoin is a no-op on a duplicate fork id", () => {
    const once = addForkJoin(base, "fork-1", "join-1", ["a", "b"], "c");
    const twice = addForkJoin(once, "fork-1", "join-2", ["a", "c"], "b");
    expect(twice.forks).toHaveLength(1);
    expect(twice.joins).toHaveLength(1);
  });

  it("addForkJoin is a no-op on a duplicate join id", () => {
    const once = addForkJoin(base, "fork-1", "join-1", ["a", "b"], "c");
    const twice = addForkJoin(once, "fork-2", "join-1", ["a", "c"], "b");
    expect(twice.forks).toHaveLength(1);
    expect(twice.joins).toHaveLength(1);
  });

  it("removeForkJoin drops both the fork and its paired join", () => {
    const d = removeForkJoin(addForkJoin(base, "fork-1", "join-1", ["a", "b"], "c"), "fork-1", "join-1");
    expect(d.forks).toHaveLength(0);
    expect(d.joins).toHaveLength(0);
  });
});
