import { describe, expect, it, vi, beforeEach } from "vitest";
import { listTemplates, listPipelines, loadPipeline } from "./pipeline";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe("pipeline ipc", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("listTemplates calls pipeline_list_templates", async () => {
    invokeMock.mockResolvedValueOnce([{ id: "ddd-spec-plan-impl", name: "DDD" }]);
    const result = await listTemplates();
    expect(invokeMock).toHaveBeenCalledWith("pipeline_list_templates");
    expect(result[0].id).toBe("ddd-spec-plan-impl");
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
