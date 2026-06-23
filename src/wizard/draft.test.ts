import { describe, expect, it } from "vitest";
import { emptyDraft, WIZARD_STEPS, renameTeam, setPromptBody, setTeamModel, addTeam, removeTeam } from "./draft";

describe("wizard draft helpers", () => {
  it("emptyDraft has no teams + current schema version", () => {
    const d = emptyDraft();
    expect(d.teams).toEqual([]);
    expect(d.schema_version).toBeGreaterThanOrEqual(2);
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
