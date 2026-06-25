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

  it("gives only the active tab tabIndex 0 (roving tabindex)", () => {
    render(<ViewSwitcher active="list" onChange={() => {}} />);
    expect(screen.getByRole("tab", { name: /list/i })).toHaveAttribute("tabindex", "0");
    expect(screen.getByRole("tab", { name: /board/i })).toHaveAttribute("tabindex", "-1");
    expect(screen.getByRole("tab", { name: /pipeline/i })).toHaveAttribute("tabindex", "-1");
  });

  it("ArrowRight activates the next tab", () => {
    const onChange = vi.fn();
    render(<ViewSwitcher active="board" onChange={onChange} />);
    fireEvent.keyDown(screen.getByRole("tab", { name: /board/i }), { key: "ArrowRight" });
    expect(onChange).toHaveBeenCalledWith("list");
  });

  it("ArrowLeft wraps from the first tab to the last", () => {
    const onChange = vi.fn();
    render(<ViewSwitcher active="board" onChange={onChange} />);
    fireEvent.keyDown(screen.getByRole("tab", { name: /board/i }), { key: "ArrowLeft" });
    expect(onChange).toHaveBeenCalledWith("settings");
  });

  it("Home/End jump to first/last tab", () => {
    const onChange = vi.fn();
    render(<ViewSwitcher active="pipeline" onChange={onChange} />);
    fireEvent.keyDown(screen.getByRole("tab", { name: /pipeline/i }), { key: "End" });
    expect(onChange).toHaveBeenCalledWith("settings");
    fireEvent.keyDown(screen.getByRole("tab", { name: /pipeline/i }), { key: "Home" });
    expect(onChange).toHaveBeenCalledWith("board");
  });

  it("ignores non-navigation keys", () => {
    const onChange = vi.fn();
    render(<ViewSwitcher active="board" onChange={onChange} />);
    fireEvent.keyDown(screen.getByRole("tab", { name: /board/i }), { key: "a" });
    expect(onChange).not.toHaveBeenCalled();
  });
});
