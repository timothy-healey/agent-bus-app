import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Terminal } from "./Terminal";
import type { Turn } from "../ipc/terminal";

const turns: Turn[] = [
  { role: "user", text: "/approve T-041", tool_calls: [], at: 1 },
  {
    role: "assistant",
    text: "Done: approve_gate.",
    tool_calls: [{ request: { tool_name: "approve_gate", args: { task_id: "T-041" } }, result: { status: "ok", result: {} } }],
    at: 1,
  },
];

describe("Terminal", () => {
  it("renders the claude head and context line", () => {
    render(<Terminal turns={[]} contextLine="full pipeline + 3 teams" onSend={vi.fn()} />);
    expect(screen.getByText(/claude/i)).toBeInTheDocument();
    expect(screen.getByText(/full pipeline \+ 3 teams/)).toBeInTheDocument();
  });

  it("renders turns and an inline tool-call chip", () => {
    render(<Terminal turns={turns} contextLine="x" onSend={vi.fn()} />);
    expect(screen.getByText("/approve T-041")).toBeInTheDocument();
    expect(screen.getByText("Done: approve_gate.")).toBeInTheDocument();
    expect(screen.getByText(/approve_gate T-041/)).toBeInTheDocument();
    expect(screen.getByText("✓")).toBeInTheDocument();
  });

  it("calls onSend with the typed input on Enter and clears the field", () => {
    const onSend = vi.fn();
    render(<Terminal turns={[]} contextLine="x" onSend={onSend} />);
    const input = screen.getByPlaceholderText(/ask, inject, approve/i) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "/inject 03-scheduling" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("/inject 03-scheduling");
    expect(input.value).toBe("");
  });

  it("renders the live streaming bubble when streaming text is present", () => {
    render(<Terminal turns={[]} contextLine="ctx" onSend={vi.fn()} streaming="typing now" />);
    expect(screen.getByText("typing now")).toBeInTheDocument();
  });

  it("renders no streaming bubble when streaming is empty", () => {
    const { container } = render(<Terminal turns={[]} contextLine="ctx" onSend={vi.fn()} streaming="" />);
    expect(container.querySelector('[data-streaming="true"]')).toBeNull();
  });

  it("collapses to a thin bar when the head toggle is clicked", () => {
    render(<Terminal turns={turns} contextLine="x" onSend={vi.fn()} />);
    const toggle = screen.getByLabelText(/collapse terminal/i);
    fireEvent.click(toggle);
    // collapsed: the stream (and its messages) is hidden
    expect(screen.queryByText("Done: approve_gate.")).not.toBeInTheDocument();
  });
});
