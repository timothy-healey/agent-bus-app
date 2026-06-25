import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const pickFolderMock = vi.fn();
vi.mock("../ipc/workspace", () => ({
  pickFolder: () => pickFolderMock(),
}));

import { FolderPickerField } from "./FolderPickerField";

describe("FolderPickerField", () => {
  beforeEach(() => pickFolderMock.mockReset());

  it("renders label + value and fires onChange on typing", () => {
    const onChange = vi.fn();
    render(<FolderPickerField label="Root path" value="~/x" onChange={onChange} />);
    expect(screen.getByText("Root path")).toBeInTheDocument();
    const input = screen.getByLabelText("Root path") as HTMLInputElement;
    expect(input.value).toBe("~/x");
    fireEvent.change(input, { target: { value: "~/y" } });
    expect(onChange).toHaveBeenCalledWith("~/y");
  });

  it("opens the NATIVE folder dialog on Browse and writes the picked path", async () => {
    pickFolderMock.mockResolvedValueOnce("/Users/tim/projects/repo");
    const onChange = vi.fn();
    render(<FolderPickerField label="Target repo" value="" onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /browse/i }));
    await waitFor(() => expect(pickFolderMock).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(onChange).toHaveBeenCalledWith("/Users/tim/projects/repo"));
  });

  it("ignores a cancelled dialog (null) without calling onChange", async () => {
    pickFolderMock.mockResolvedValueOnce(null);
    const onChange = vi.fn();
    render(<FolderPickerField label="Root path" value="~/x" onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /browse/i }));
    await waitFor(() => expect(pickFolderMock).toHaveBeenCalledTimes(1));
    expect(onChange).not.toHaveBeenCalled();
  });
});
