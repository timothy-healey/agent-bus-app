import { describe, expect, it, vi, beforeEach } from "vitest";
import { writeProjectPipeline } from "./workspace";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe("writeProjectPipeline ipc", () => {
  beforeEach(() => invokeMock.mockReset());

  it("passes project_id, yaml path, yaml + prompts", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await writeProjectPipeline("proj-1", "pipelines/demo.yaml", "id: demo\n", [["prompts/a.md", "body"]]);
    expect(invokeMock).toHaveBeenCalledWith("write_project_pipeline", {
      project_id: "proj-1",
      yaml_rel_path: "pipelines/demo.yaml",
      pipeline_yaml: "id: demo\n",
      prompts: [["prompts/a.md", "body"]],
    });
  });
});
