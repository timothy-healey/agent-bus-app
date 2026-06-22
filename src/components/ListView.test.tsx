import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ListView } from "./ListView";
import type { Task } from "../ipc/runtime";

function t(over: Partial<Task>): Task {
  return {
    id: "T-1", project_id: "p", pipeline: "pipe", topic: "Bulk write",
    target_repo: null, target_scope: null, current_stage: "research",
    state: "queued", attempts: 1, parent_artifact: null, review_artifact: null,
    created_at: 0, updated_at: 0, ...over,
  };
}

const tasks = [
  t({ id: "T-1", state: "gated", topic: "alpha", current_stage: "gate-1" }),
  t({ id: "T-2", state: "running", topic: "beta", current_stage: "research" }),
];

describe("ListView", () => {
  it("renders a row per task with id and topic", () => {
    render(<ListView tasks={tasks} tokensByTask={{}} now={100} onOpenCard={() => {}} />);
    expect(screen.getByText("T-1")).toBeInTheDocument();
    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(screen.getByText("beta")).toBeInTheDocument();
  });

  it("filters when a pill is clicked", () => {
    render(<ListView tasks={tasks} tokensByTask={{}} now={100} onOpenCard={() => {}} />);
    fireEvent.click(screen.getByText("running"));
    expect(screen.queryByText("alpha")).not.toBeInTheDocument();
    expect(screen.getByText("beta")).toBeInTheDocument();
  });

  it("filters by search box", () => {
    render(<ListView tasks={tasks} tokensByTask={{}} now={100} onOpenCard={() => {}} />);
    fireEvent.change(screen.getByPlaceholderText(/filter tasks/i), { target: { value: "alpha" } });
    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(screen.queryByText("beta")).not.toBeInTheDocument();
  });

  it("calls onOpenCard with the task id on row click", () => {
    const onOpen = vi.fn();
    render(<ListView tasks={tasks} tokensByTask={{}} now={100} onOpenCard={onOpen} />);
    fireEvent.click(screen.getByText("alpha"));
    expect(onOpen).toHaveBeenCalledWith("T-1");
  });

  it("shows an empty state when no tasks match", () => {
    render(<ListView tasks={[]} tokensByTask={{}} now={100} onOpenCard={() => {}} />);
    expect(screen.getByText(/no tasks/i)).toBeInTheDocument();
  });
});
