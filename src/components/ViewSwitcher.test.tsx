import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ViewSwitcher } from "./ViewSwitcher";

describe("ViewSwitcher", () => {
  it("renders board + pipeline tabs", () => {
    render(<ViewSwitcher active="pipeline" onChange={() => {}} />);
    expect(screen.getByRole("tab", { name: /board/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /pipeline/i })).toBeInTheDocument();
  });

  it("marks the active tab with aria-selected", () => {
    render(<ViewSwitcher active="pipeline" onChange={() => {}} />);
    expect(screen.getByRole("tab", { name: /pipeline/i })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: /board/i })).toHaveAttribute("aria-selected", "false");
  });

  it("calls onChange when a tab is clicked", () => {
    const onChange = vi.fn();
    render(<ViewSwitcher active="board" onChange={onChange} />);
    fireEvent.click(screen.getByRole("tab", { name: /pipeline/i }));
    expect(onChange).toHaveBeenCalledWith("pipeline");
  });
});
