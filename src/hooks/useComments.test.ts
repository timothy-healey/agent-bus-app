import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";

const listMock = vi.fn();
const addMock = vi.fn();
const delMock = vi.fn();
const reanchorMock = vi.fn();

vi.mock("../ipc/review", () => ({
  listComments: (...a: unknown[]) => listMock(...a),
  addComment: (...a: unknown[]) => addMock(...a),
  deleteComment: (...a: unknown[]) => delMock(...a),
  reanchorComments: (...a: unknown[]) => reanchorMock(...a),
}));

import { useComments } from "./useComments";

describe("useComments", () => {
  beforeEach(() => {
    listMock.mockReset();
    addMock.mockReset();
    delMock.mockReset();
    reanchorMock.mockReset();
  });

  it("loads comments for the task on mount", async () => {
    listMock.mockResolvedValueOnce([{ id: "c1", note: "x", kind: "inline" }]);
    const { result } = renderHook(() => useComments("T-1", "a.md"));
    await waitFor(() => expect(result.current.comments).toHaveLength(1));
    expect(listMock).toHaveBeenCalledWith("T-1");
  });

  it("add inserts then reloads", async () => {
    listMock.mockResolvedValueOnce([]);
    const { result } = renderHook(() => useComments("T-1", "a.md"));
    await waitFor(() => expect(result.current.comments).toHaveLength(0));

    addMock.mockResolvedValueOnce({ id: "c1" });
    listMock.mockResolvedValueOnce([{ id: "c1", note: "n", kind: "inline" }]);
    await act(async () => {
      await result.current.add({ note: "n", anchorText: "span", anchorOffset: 3, kind: "inline" });
    });
    expect(addMock).toHaveBeenCalledWith({
      taskId: "T-1",
      artifactPath: "a.md",
      note: "n",
      anchorText: "span",
      anchorOffset: 3,
      kind: "inline",
    });
    await waitFor(() => expect(result.current.comments).toHaveLength(1));
  });

  it("remove deletes then reloads", async () => {
    listMock.mockResolvedValueOnce([{ id: "c1", note: "n", kind: "inline" }]);
    const { result } = renderHook(() => useComments("T-1", "a.md"));
    await waitFor(() => expect(result.current.comments).toHaveLength(1));

    delMock.mockResolvedValueOnce(undefined);
    listMock.mockResolvedValueOnce([]);
    await act(async () => {
      await result.current.remove("c1");
    });
    expect(delMock).toHaveBeenCalledWith("c1");
    await waitFor(() => expect(result.current.comments).toHaveLength(0));
  });

  it("is a no-op for a synthetic gen: id (no list/add/persist)", async () => {
    const { result } = renderHook(() => useComments("gen:R-1:research", "artifacts/gen:R-1:research.md"));
    await waitFor(() => expect(result.current.comments).toHaveLength(0));
    expect(listMock).not.toHaveBeenCalled();
    await act(async () => {
      await result.current.add({ note: "n", kind: "inline" });
    });
    expect(addMock).not.toHaveBeenCalled();
  });

  it("reanchored falls back to status open at stored offset with no version markdown", async () => {
    listMock.mockResolvedValueOnce([
      { id: "c1", anchor_offset: 5, note: "n", kind: "inline" },
    ]);
    const { result } = renderHook(() => useComments("T-1", "a.md"));
    await waitFor(() => expect(result.current.reanchored).toHaveLength(1));
    expect(result.current.reanchored[0].status).toBe("open");
    expect(result.current.reanchored[0].effective_offset).toBe(5);
    expect(reanchorMock).not.toHaveBeenCalled();
  });

  it("reanchored uses reanchorComments when version markdown is provided", async () => {
    listMock.mockResolvedValueOnce([
      { id: "c1", anchor_offset: 5, note: "n", kind: "inline" },
    ]);
    reanchorMock.mockResolvedValueOnce([
      { id: "c1", anchor_offset: 5, note: "n", kind: "inline", status: "addressed", effective_offset: 0 },
    ]);
    const { result } = renderHook(() =>
      useComments("T-1", "a.md", "<!-- addressed: c1 -->"),
    );
    await waitFor(() => expect(result.current.reanchored[0]?.status).toBe("addressed"));
    expect(reanchorMock).toHaveBeenCalledWith("T-1", "<!-- addressed: c1 -->");
  });
});
