import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const listDirMock = vi.fn();
vi.mock("../ipc/workspace", () => ({
  listDir: (path: string) => listDirMock(path),
}));

import { FileTreePicker } from "./FileTreePicker";

beforeEach(() => listDirMock.mockReset());

describe("FileTreePicker", () => {
  it("loads + renders the root's children dirs-first via list_dir", async () => {
    listDirMock.mockResolvedValueOnce([
      { name: "src", path: "/r/src", is_dir: true },
      { name: "a.txt", path: "/r/a.txt", is_dir: false },
    ]);
    render(<FileTreePicker root="/r" mode="files" selected={[]} onChange={vi.fn()} />);
    await waitFor(() => expect(screen.getByText(/src/)).toBeInTheDocument());
    expect(listDirMock).toHaveBeenCalledWith("/r");
    expect(screen.getByRole("tree")).toBeInTheDocument();
  });

  it("multi-select mode toggles files + folders via checkbox", async () => {
    listDirMock.mockResolvedValueOnce([
      { name: "a.txt", path: "/r/a.txt", is_dir: false },
    ]);
    const onChange = vi.fn();
    render(<FileTreePicker root="/r" mode="files" selected={[]} onChange={onChange} />);
    await waitFor(() => expect(screen.getByText(/a\.txt/)).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText("select a.txt"));
    expect(onChange).toHaveBeenCalledWith(["/r/a.txt"]);
  });

  it("folder mode does not select files and shows no checkbox", async () => {
    listDirMock.mockResolvedValueOnce([
      { name: "a.txt", path: "/r/a.txt", is_dir: false },
      { name: "src", path: "/r/src", is_dir: true },
    ]);
    const onChange = vi.fn();
    render(<FileTreePicker root="/r" mode="folder" selected={[]} onChange={onChange} />);
    await waitFor(() => expect(screen.getByText(/a\.txt/)).toBeInTheDocument());
    // no checkboxes in folder mode
    expect(screen.queryByLabelText("select a.txt")).not.toBeInTheDocument();
    // clicking a file does nothing; clicking a folder selects it
    fireEvent.click(screen.getByText(/a\.txt/));
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText(/src/));
    expect(onChange).toHaveBeenCalledWith(["/r/src"]);
  });

  it("lazily expands a directory on the twisty", async () => {
    listDirMock
      .mockResolvedValueOnce([{ name: "src", path: "/r/src", is_dir: true }])
      .mockResolvedValueOnce([{ name: "main.rs", path: "/r/src/main.rs", is_dir: false }]);
    render(<FileTreePicker root="/r" mode="files" selected={[]} onChange={vi.fn()} />);
    await waitFor(() => expect(screen.getByText(/src/)).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: /expand src/i }));
    await waitFor(() => expect(screen.getByText(/main\.rs/)).toBeInTheDocument());
    expect(listDirMock).toHaveBeenCalledWith("/r/src");
  });

  it("surfaces a list_dir error in the tree (not a crash)", async () => {
    listDirMock.mockRejectedValueOnce("cannot read /r: denied");
    render(<FileTreePicker root="/r" mode="files" selected={[]} onChange={vi.fn()} />);
    await waitFor(() => expect(screen.getByText(/cannot read/)).toBeInTheDocument());
  });
});
