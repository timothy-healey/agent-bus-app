import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ViewSwitcher } from "./ViewSwitcher";

describe("ViewSwitcher", () => {
  it("renders board, list, pipeline, settings tabs", () => {
    render(<ViewSwitcher active="board" onChange={() => {}} />);
    for (const name of [/board/i, /list/i, /pipeline/i, /settings/i]) {
      expect(screen.getByRole("tab", { name })).toBeInTheDocument();
    }
  });

  it("marks the active tab with aria-selected", () => {
    render(<ViewSwitcher active="list" onChange={() => {}} />);
    expect(screen.getByRole("tab", { name: /list/i })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: /board/i })).toHaveAttribute("aria-selected", "false");
  });

  it("calls onChange when list is clicked", () => {
    const onChange = vi.fn();
    render(<ViewSwitcher active="board" onChange={onChange} />);
    fireEvent.click(screen.getByRole("tab", { name: /list/i }));
    expect(onChange).toHaveBeenCalledWith("list");
  });

  it("right-aligns the settings tab", () => {
    render(<ViewSwitcher active="board" onChange={() => {}} />);
    const settings = screen.getByRole("tab", { name: /settings/i });
    expect(settings.style.marginLeft).toBe("auto");
  });
});
