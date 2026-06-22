import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { SelectionPopover } from "./SelectionPopover";

describe("SelectionPopover", () => {
  it("shows the quoted selection", () => {
    render(
      <SelectionPopover
        quote="idempotency key at the batch level"
        onAdd={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.getByText(/idempotency key at the batch level/)).toBeInTheDocument();
  });

  it("calls onAdd with the typed note", () => {
    const onAdd = vi.fn();
    render(<SelectionPopover quote="x" onAdd={onAdd} onCancel={() => {}} />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "per-row, not per-batch" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add comment/i }));
    expect(onAdd).toHaveBeenCalledWith("per-row, not per-batch");
  });

  it("does not call onAdd for an empty note", () => {
    const onAdd = vi.fn();
    render(<SelectionPopover quote="x" onAdd={onAdd} onCancel={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /add comment/i }));
    expect(onAdd).not.toHaveBeenCalled();
  });

  it("calls onCancel", () => {
    const onCancel = vi.fn();
    render(<SelectionPopover quote="x" onAdd={() => {}} onCancel={onCancel} />);
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(onCancel).toHaveBeenCalledOnce();
  });
});
