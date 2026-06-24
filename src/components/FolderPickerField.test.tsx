import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const pickFolderMock = vi.fn();
vi.mock("../ipc/workspace", () => ({
  pickFolder: () => pickFolderMock(),
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

  it("writes the picked absolute path into the field via Browse (mocked dialog)", async () => {
    pickFolderMock.mockResolvedValueOnce("/Users/tim/projects/example");
    const onChange = vi.fn();
    render(<FolderPickerField label="Target repo" value="" onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /browse/i }));
    await waitFor(() =>
      expect(onChange).toHaveBeenCalledWith("/Users/tim/projects/example"),
    );
  });

  it("does nothing when the picker is cancelled (null)", async () => {
    pickFolderMock.mockResolvedValueOnce(null);
    const onChange = vi.fn();
    render(<FolderPickerField label="Target repo" value="keep" onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /browse/i }));
    await waitFor(() => expect(pickFolderMock).toHaveBeenCalled());
    expect(onChange).not.toHaveBeenCalled();
  });
});
