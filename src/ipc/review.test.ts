import { describe, expect, it, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  addComment,
  listComments,
  deleteComment,
  recordVerdict,
} from "./review";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe("review ipc", () => {
  beforeEach(() => invokeMock.mockReset());

  it("addComment passes anchor + kind with snake_case keys", async () => {
    invokeMock.mockResolvedValueOnce({ id: "c1", kind: "inline" });
    await addComment({
      taskId: "T-1",
      artifactPath: "artifacts/specs/T-1-v1.md",
      note: "per-row",
      anchorText: "key",
      anchorOffset: 12,
      kind: "inline",
    });
    expect(invokeMock).toHaveBeenCalledWith("add_comment", {
      task_id: "T-1",
      artifact_path: "artifacts/specs/T-1-v1.md",
      note: "per-row",
      anchor_text: "key",
      anchor_offset: 12,
      kind: "inline",
    });
  });

  it("addComment nulls a missing anchor", async () => {
    invokeMock.mockResolvedValueOnce({ id: "c2", kind: "direction" });
    await addComment({
      taskId: "T-1",
      artifactPath: "a.md",
      note: "overall",
      kind: "direction",
    });
    expect(invokeMock).toHaveBeenCalledWith("add_comment", {
      task_id: "T-1",
      artifact_path: "a.md",
      note: "overall",
      anchor_text: null,
      anchor_offset: null,
      kind: "direction",
    });
  });

  it("listComments passes task_id", async () => {
    invokeMock.mockResolvedValueOnce([]);
    await listComments("T-1");
    expect(invokeMock).toHaveBeenCalledWith("list_comments", { task_id: "T-1" });
  });

  it("deleteComment passes comment_id", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await deleteComment("c1");
    expect(invokeMock).toHaveBeenCalledWith("delete_comment", {
      comment_id: "c1",
    });
  });

  it("recordVerdict passes task_id + verdict", async () => {
    invokeMock.mockResolvedValueOnce({
      task_id: "T-1",
      verdict: "revise",
      comment_count: 2,
    });
    const m = await recordVerdict("T-1", "revise");
    expect(invokeMock).toHaveBeenCalledWith("record_verdict", {
      task_id: "T-1",
      verdict: "revise",
    });
    expect(m.comment_count).toBe(2);
  });
});
