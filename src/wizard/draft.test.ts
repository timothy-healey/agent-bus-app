import { describe, expect, it } from "vitest";
import { emptyDraft, WIZARD_STEPS, renameTeam, setPromptBody, setTeamModel, addTeam, removeTeam } from "./draft";
import { setTeamEffort, setTeamReads, setTeamWrites } from "./draft";
import { setTeamGrant, setTeamBashPatterns, bashPatterns, setTeamPlugin, addTeamPlugin } from "./draft";
import { addGate, removeGate, setTeamApprove } from "./draft";
import { addForkJoin, removeForkJoin } from "./draft";
import { setJoinQuorum, setJoinCancelOnReject } from "./draft";
import { setTeamRole, setTeamStoreCapacity, setTeamWorkers } from "./draft";
import { draftToPipeline } from "./draftToPipeline";
import type { ModelList } from "../ipc/models";

const LIST: ModelList = {
  source: "live",
  cli_version: null,
  models: [
    { value: "opus", resolved_model: "claude-opus-5-5", display_name: "Opus 5.5", description: null, effort_levels: ["low", "high", "xhigh"], supports_auto_mode: true },
    { value: "claude-opus-4-6", resolved_model: "claude-opus-4-6", display_name: "Opus 4.6", description: null, effort_levels: ["low", "high"], supports_auto_mode: true },
    { value: "haiku", resolved_model: "claude-haiku-4-5-20251001", display_name: "Haiku 4.5", description: null, effort_levels: [], supports_auto_mode: false },
  ],
};

describe("wizard draft helpers", () => {
  it("emptyDraft has no teams + current schema version", () => {
    const d = emptyDraft();
    expect(d.teams).toEqual([]);
    expect(d.schema_version).toBeGreaterThanOrEqual(2);
  });

  it("addTeam defaults to the default alias with Default effort", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    expect(d.teams[0].runner.model).toBe("default");
    expect(d.teams[0].runner.effort).toBeUndefined();
    expect("effort" in d.teams[0].runner).toBe(false);
  });

  it("emptyDraft seeds an empty gates array", () => {
    expect(emptyDraft().gates).toEqual([]);
  });

  it("WIZARD_STEPS lists basics → canvas → review in order", () => {
    expect(WIZARD_STEPS).toEqual(["basics", "canvas", "review"]);
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
    const { draft, snapped } = setTeamModel(addTeam(emptyDraft(), "research", "Research"), "research", "haiku", LIST);
    expect(draft.teams[0].runner.model).toBe("haiku");
    expect(snapped).toBeNull();
  });

  it("switching to a model without the saved level snaps effort to Default", () => {
    const base = setTeamEffort(setTeamModel(addTeam(emptyDraft(), "r", "R"), "r", "opus", LIST).draft, "r", "xhigh");
    const { draft, snapped } = setTeamModel(base, "r", "claude-opus-4-6", LIST);
    expect(draft.teams[0].runner.model).toBe("claude-opus-4-6");
    expect(draft.teams[0].runner.effort).toBeUndefined();
    expect(snapped).toBe("xhigh");
  });

  it("switching to a model that supports the level keeps it", () => {
    const base = setTeamEffort(addTeam(emptyDraft(), "r", "R"), "r", "high");
    const { draft, snapped } = setTeamModel(base, "r", "claude-opus-4-6", LIST);
    expect(draft.teams[0].runner.effort).toBe("high");
    expect(snapped).toBeNull();
  });

  it("a model without effort support snaps any level", () => {
    const base = setTeamEffort(addTeam(emptyDraft(), "r", "R"), "r", "low");
    expect(setTeamModel(base, "r", "haiku", LIST).snapped).toBe("low");
  });

  it("without a list nothing snaps", () => {
    const base = setTeamEffort(addTeam(emptyDraft(), "r", "R"), "r", "low");
    const { draft, snapped } = setTeamModel(base, "r", "haiku", null);
    expect(snapped).toBeNull();
    expect(draft.teams[0].runner.effort).toBe("low");
  });

  it("removeTeam drops the team", () => {
    const d = removeTeam(addTeam(emptyDraft(), "research", "Research"), "research");
    expect(d.teams).toHaveLength(0);
  });
});

describe("advanced team config helpers (W2)", () => {
  const base = addTeam(emptyDraft(), "research", "Research");

  it("setTeamEffort sets a level", () => {
    const d = setTeamEffort(base, "research", "high");
    expect(d.teams[0].runner.effort).toBe("high");
  });

  it("setTeamEffort(undefined) clears the level so none is sent", () => {
    const d = setTeamEffort(setTeamEffort(base, "research", "low"), "research", undefined);
    expect("effort" in d.teams[0].runner).toBe(false);
    expect(JSON.stringify(d.teams[0].runner)).not.toContain("effort");
  });

  it("a new team starts with no grants and no plugins", () => {
    expect(base.teams[0].scope.grants).toEqual([]);
    expect(base.teams[0].scope.plugins).toEqual([]);
  });

  it("setTeamGrant toggles one grant without duplicating it", () => {
    let d = setTeamGrant(base, "research", "remote-git", true);
    d = setTeamGrant(d, "research", "remote-git", true);
    d = setTeamGrant(d, "research", "agent", true);
    expect(d.teams[0].scope.grants).toEqual(["remote-git", "agent"]);
    d = setTeamGrant(d, "research", "remote-git", false);
    expect(d.teams[0].scope.grants).toEqual(["agent"]);
  });

  it("setTeamBashPatterns replaces the pattern grants and keeps the others", () => {
    let d = setTeamGrant(base, "research", "agent", true);
    d = setTeamBashPatterns(d, "research", "git diff:*, Bash(git log:*) ,, ");
    expect(d.teams[0].scope.grants).toEqual(["agent", "bash(git diff:*)", "bash(git log:*)"]);
    expect(bashPatterns(d.teams[0].scope)).toEqual(["git diff:*", "git log:*"]);
    d = setTeamBashPatterns(d, "research", "");
    expect(d.teams[0].scope.grants).toEqual(["agent"]);
  });

  it("setTeamPlugin and addTeamPlugin add each plugin once", () => {
    let d = addTeamPlugin(base, "research", "superpowers");
    d = addTeamPlugin(d, "research", "superpowers");
    d = setTeamPlugin(d, "research", "ddd-council", true);
    expect(d.teams[0].scope.plugins).toEqual(["superpowers", "ddd-council"]);
    d = setTeamPlugin(d, "research", "superpowers", false);
    expect(d.teams[0].scope.plugins).toEqual(["ddd-council"]);
  });

  it("setTeamReads / setTeamWrites set scope.reads / scope.writes", () => {
    const d1 = setTeamReads(base, "research", "src/**, docs/**");
    expect(d1.teams[0].scope.reads).toEqual(["src/**", "docs/**"]);
    const d2 = setTeamWrites(base, "research", "artifacts/**");
    expect(d2.teams[0].scope.writes).toEqual(["artifacts/**"]);
  });
});

describe("role / scale / store helpers (chunk ②)", () => {
  const base = addTeam(emptyDraft(), "research", "Research");

  it("defaultTeam seeds role=producer and a store capacity", () => {
    expect(base.teams[0].role).toBe("producer");
    expect(base.teams[0].store?.capacity).toBeGreaterThan(0);
  });

  it("setTeamRole flips a team's role", () => {
    const d = setTeamRole(base, "research", "reviewer");
    expect(d.teams[0].role).toBe("reviewer");
    // immutable: the source draft is untouched
    expect(base.teams[0].role).toBe("producer");
  });

  it("setTeamRole only touches the named team", () => {
    const two = addTeam(base, "review", "Review");
    const d = setTeamRole(two, "review", "reviewer");
    expect(d.teams.find((t) => t.id === "research")?.role).toBe("producer");
    expect(d.teams.find((t) => t.id === "review")?.role).toBe("reviewer");
  });

  it("setTeamStoreCapacity sets the WIP capacity", () => {
    const d = setTeamStoreCapacity(base, "research", 12);
    expect(d.teams[0].store).toEqual({ capacity: 12 });
  });

  it("setTeamStoreCapacity floors at 1 and coerces non-finite to 1", () => {
    expect(setTeamStoreCapacity(base, "research", 0).teams[0].store?.capacity).toBe(1);
    expect(setTeamStoreCapacity(base, "research", -5).teams[0].store?.capacity).toBe(1);
    expect(setTeamStoreCapacity(base, "research", Number.NaN).teams[0].store?.capacity).toBe(1);
  });

  it("setTeamWorkers sets the Scale min/max", () => {
    const d = setTeamWorkers(base, "research", { min: 2, max: 5 });
    expect(d.teams[0].workers).toEqual({ min: 2, max: 5 });
  });

  it("setTeamWorkers clamps min>=1 and max>=min", () => {
    expect(setTeamWorkers(base, "research", { min: 0, max: 3 }).teams[0].workers).toEqual({ min: 1, max: 3 });
    // max below min is raised to min
    expect(setTeamWorkers(base, "research", { min: 4, max: 2 }).teams[0].workers).toEqual({ min: 4, max: 4 });
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

describe("join field mutators (AU1 — quorum + cancel-on-reject)", () => {
  const base = addForkJoin(
    addTeam(addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B"), "c", "C"),
    "fork-1",
    "join-1",
    ["a", "b"],
    "c",
  );

  it("addForkJoin seeds a join WITHOUT quorum/cancel_on_reject (both start undefined)", () => {
    expect(base.joins[0].quorum).toBeUndefined();
    expect(base.joins[0].cancel_on_reject).toBeUndefined();
  });

  it("setJoinQuorum writes the number on the matching join", () => {
    const d = setJoinQuorum(base, "join-1", 2);
    expect(d.joins[0].quorum).toBe(2);
  });

  it("setJoinQuorum with undefined clears it back to all-must-approve", () => {
    const set = setJoinQuorum(base, "join-1", 2);
    const cleared = setJoinQuorum(set, "join-1", undefined);
    expect(cleared.joins[0].quorum).toBeUndefined();
  });

  it("setJoinQuorum keeps cancel_on_reject intact (it is merely ignored by the runtime)", () => {
    const withToggle = setJoinCancelOnReject(base, "join-1", true);
    const withQuorum = setJoinQuorum(withToggle, "join-1", 2);
    expect(withQuorum.joins[0].cancel_on_reject).toBe(true);
    // clearing the quorum restores the prior toggle untouched
    expect(setJoinQuorum(withQuorum, "join-1", undefined).joins[0].cancel_on_reject).toBe(true);
  });

  it("setJoinQuorum is a no-op on an unknown join id", () => {
    const d = setJoinQuorum(base, "ghost", 2);
    expect(d).toEqual(base);
  });

  it("setJoinCancelOnReject writes the boolean on the matching join", () => {
    expect(setJoinCancelOnReject(base, "join-1", true).joins[0].cancel_on_reject).toBe(true);
    expect(setJoinCancelOnReject(base, "join-1", false).joins[0].cancel_on_reject).toBe(false);
  });

  it("setJoinCancelOnReject is a no-op on an unknown join id", () => {
    const d = setJoinCancelOnReject(base, "ghost", true);
    expect(d).toEqual(base);
  });

  it("both fields round-trip through draftToPipeline (adapter passes joins straight through)", () => {
    const d = setJoinCancelOnReject(setJoinQuorum(base, "join-1", 2), "join-1", true);
    const p = draftToPipeline(d);
    expect(p.joins[0].quorum).toBe(2);
    expect(p.joins[0].cancel_on_reject).toBe(true);
  });
});
