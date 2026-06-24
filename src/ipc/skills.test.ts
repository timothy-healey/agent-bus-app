import { describe, expect, it, vi, beforeEach } from "vitest";
import { insertForm, listSkills, type SkillEntry } from "./skills";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

function entry(over: Partial<SkillEntry> = {}): SkillEntry {
  return {
    name: "brainstorming",
    kind: "skill",
    namespace: "superpowers",
    description: "Explore intent",
    verbs: [],
    source: "global",
    qualified: false,
    ...over,
  };
}

describe("skills ipc", () => {
  beforeEach(() => invokeMock.mockReset());

  it("listSkills calls list_skills with snake_case project_id", async () => {
    const rows: SkillEntry[] = [entry()];
    invokeMock.mockResolvedValueOnce(rows);
    const result = await listSkills("proj-1");
    expect(invokeMock).toHaveBeenCalledWith("list_skills", { project_id: "proj-1" });
    expect(result).toEqual(rows);
  });

  it("insertForm returns the bare name when not qualified", () => {
    expect(insertForm(entry())).toBe("brainstorming");
  });

  it("insertForm returns namespace:name when qualified + namespaced", () => {
    expect(insertForm(entry({ qualified: true }))).toBe("superpowers:brainstorming");
  });

  it("insertForm stays bare when qualified but no namespace", () => {
    expect(insertForm(entry({ qualified: true, namespace: null }))).toBe("brainstorming");
  });
});
