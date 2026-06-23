import { describe, expect, it, vi, beforeEach } from "vitest";
import { listPipelines, loadPipeline, kickoffGenerate, designSessionTurn, bestEffortValidate } from "./pipeline";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe("pipeline ipc", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("kickoffGenerate passes session_id + description", async () => {
    const draft = { id: "p", name: "P", description: "d", schema_version: 2, teams: [], forks: [], joins: [], escalations: [] };
    invokeMock.mockResolvedValueOnce(draft);
    const result = await kickoffGenerate("s1", "build a flow");
    expect(invokeMock).toHaveBeenCalledWith("kickoff_generate_cmd", { session_id: "s1", description: "build a flow" });
    expect(result.id).toBe("p");
  });

  it("designSessionTurn passes step + draft + user_message and returns prose + issues", async () => {
    const draft = { id: "p", name: "P", description: "", schema_version: 2, teams: [], gates: [], forks: [], joins: [], escalations: [] };
    invokeMock.mockResolvedValueOnce({ reply_text: "ok", updated_draft: draft, issues: ["draft has no teams yet"] });
    const out = await designSessionTurn("s1", "teams", draft, "add a team");
    expect(invokeMock).toHaveBeenCalledWith("design_session_turn_cmd", {
      session_id: "s1", step: "teams", draft, user_message: "add a team",
    });
    expect(out.reply_text).toBe("ok");
    expect(out.issues).toEqual(["draft has no teams yet"]);
  });

  it("bestEffortValidate passes the draft and returns the issues list", async () => {
    const draft = { id: "p", name: "P", description: "", schema_version: 2, teams: [], gates: [], forks: [], joins: [], escalations: [] };
    invokeMock.mockResolvedValueOnce(["draft has no teams yet"]);
    const issues = await bestEffortValidate(draft);
    expect(invokeMock).toHaveBeenCalledWith("best_effort_validate_cmd", { draft });
    expect(issues).toEqual(["draft has no teams yet"]);
  });

  it("listPipelines passes project_root", async () => {
    invokeMock.mockResolvedValueOnce(["ddd-spec-plan-impl"]);
    const result = await listPipelines("/p");
    expect(invokeMock).toHaveBeenCalledWith("pipeline_list", { project_root: "/p" });
    expect(result).toEqual(["ddd-spec-plan-impl"]);
  });

  it("loadPipeline passes project_root + id and returns the graph", async () => {
    const graph = {
      id: "ddd-spec-plan-impl",
      name: "DDD",
      description: "",
      schema_version: 2,
      teams: [],
      gates: [],
      escalations: [],
      forks: [{ id: "fork-1", lanes: ["a", "b"] }],
      joins: [{ id: "join-1", waits_for: ["a", "b"], downstream: "after" }],
    };
    invokeMock.mockResolvedValueOnce(graph);
    const result = await loadPipeline("/p", "ddd-spec-plan-impl");
    expect(invokeMock).toHaveBeenCalledWith("pipeline_load", {
      project_root: "/p",
      id: "ddd-spec-plan-impl",
    });
    expect(result.forks[0].lanes).toEqual(["a", "b"]);
    expect(result.joins[0].downstream).toBe("after");
  });
});
