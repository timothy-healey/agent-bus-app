import { describe, expect, it, vi, beforeEach } from "vitest";
import { createProject, listProjects } from "./workspace";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe("workspace ipc", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("createProject calls workspace_create_project with name + root_path", async () => {
    const project = {
      id: "proj-abc",
      name: "Demo",
      root_path: "/tmp/demo",
      active_pipeline_id: null,
      created_at: 1,
      updated_at: 1,
    };
    invokeMock.mockResolvedValueOnce(project);

    const result = await createProject({ name: "Demo", root_path: "/tmp/demo" });

    expect(invokeMock).toHaveBeenCalledWith("workspace_create_project", {
      name: "Demo",
      root_path: "/tmp/demo",
    });
    expect(result).toEqual(project);
  });

  it("listProjects calls workspace_list_projects and returns array", async () => {
    invokeMock.mockResolvedValueOnce([]);
    const result = await listProjects();
    expect(invokeMock).toHaveBeenCalledWith("workspace_list_projects");
    expect(result).toEqual([]);
  });
});
