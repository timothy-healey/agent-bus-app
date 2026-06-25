import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { useSkillCatalog } from "./useSkillCatalog";
import type { SkillEntry } from "../ipc/skills";

const listSkillsMock = vi.fn();
vi.mock("../ipc/skills", () => ({
  listSkills: (id: string) => listSkillsMock(id),
}));

function entry(name: string): SkillEntry {
  return {
    name,
    kind: "skill",
    namespace: null,
    description: "",
    verbs: [],
    source: "global",
    qualified: false,
  };
}

describe("useSkillCatalog", () => {
  beforeEach(() => listSkillsMock.mockReset());

  it("loads the catalog for the active project on mount", async () => {
    listSkillsMock.mockResolvedValueOnce([entry("a"), entry("b")]);
    const { result } = renderHook(() => useSkillCatalog("proj-1"));
    await waitFor(() => expect(result.current.entries.length).toBe(2));
    expect(listSkillsMock).toHaveBeenCalledWith("proj-1");
  });

  it("loads the GLOBAL catalog when no project is selected (G4)", async () => {
    // G4: with no project (the new-project wizard), the hook must still load the
    // global ~/.claude catalog — it does NOT short-circuit to empty.
    listSkillsMock.mockResolvedValueOnce([entry("ddd-council")]);
    const { result } = renderHook(() => useSkillCatalog(null));
    await waitFor(() => expect(result.current.entries.length).toBe(1));
    expect(listSkillsMock).toHaveBeenCalledWith(null);
  });

  it("refresh re-scans", async () => {
    listSkillsMock.mockResolvedValueOnce([entry("a")]).mockResolvedValueOnce([entry("a"), entry("c")]);
    const { result } = renderHook(() => useSkillCatalog("proj-1"));
    await waitFor(() => expect(result.current.entries.length).toBe(1));
    act(() => result.current.refresh());
    await waitFor(() => expect(result.current.entries.length).toBe(2));
    expect(listSkillsMock).toHaveBeenCalledTimes(2);
  });

  it("degrades to an empty catalog on a discovery failure", async () => {
    listSkillsMock.mockRejectedValueOnce(new Error("scan failed"));
    const { result } = renderHook(() => useSkillCatalog("proj-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.entries).toEqual([]);
  });
});
