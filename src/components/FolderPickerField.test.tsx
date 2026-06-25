import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const listDirMock = vi.fn();
vi.mock("../ipc/workspace", () => ({
  listDir: (path: string) => listDirMock(path),
}));

import { FolderPickerField } from "./FolderPickerField";

describe("FolderPickerField", () => {
  it("renders label + value and fires onChange on typing", () => {
    const onChange = vi.fn();
    render(<FolderPickerField label="Root path" value="~/x" onChange={onChange} />);
    expect(screen.getByText("Root path")).toBeInTheDocument();
    const input = screen.getByLabelText("Root path") as HTMLInputElement;
    expect(input.value).toBe("~/x");
    fireEvent.change(input, { target: { value: "~/y" } });
    expect(onChange).toHaveBeenCalledWith("~/y");
  });

  it("opens the in-app tree on Browse and writes the picked folder", async () => {
    listDirMock.mockResolvedValueOnce([
      { name: "projects", path: "/Users/tim/projects", is_dir: true },
      { name: "readme.md", path: "/Users/tim/readme.md", is_dir: false },
    ]);
    const onChange = vi.fn();
    render(<FolderPickerField label="Target repo" value="" onChange={onChange} root="~" />);
    fireEvent.click(screen.getByRole("button", { name: /browse/i }));
    // the tree loaded the root and shows the folder
    await waitFor(() => expect(screen.getByText(/projects/)).toBeInTheDocument());
    // a file is not selectable in folder mode (no checkbox, click is a no-op)
    fireEvent.click(screen.getByText(/projects/));
    expect(onChange).toHaveBeenCalledWith("/Users/tim/projects");
  });
});
