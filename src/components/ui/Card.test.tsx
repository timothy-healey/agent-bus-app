import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Card } from "./Card";
import type { Task } from "../../ipc/runtime";

function task(over: Partial<Task> = {}): Task {
  return {
    id: "T-42",
    project_id: "p",
    pipeline: "p",
    topic: "Patient Records bulk-write",
    target_repo: null,
    target_scope: null,
    current_stage: "research",
    state: "running",
    attempts: 1,
    parent_artifact: null,
    review_artifact: null,
    created_at: 0,
    updated_at: 0,
    run_id: null,
    item_key: "patient-records",
    ...over,
  };
}

describe("Card", () => {
  it("shows the slug, topic, attempts and state label (not the raw id)", () => {
    render(<Card task={task()} tokens={47_000} />);
    // LF32: the slug is the secondary mono metadata; the raw task id is gone.
    expect(screen.getByText("patient-records")).toBeInTheDocument();
    expect(screen.queryByText("T-42")).not.toBeInTheDocument();
    expect(screen.getByText("Patient Records bulk-write")).toBeInTheDocument();
    expect(screen.getByText("running")).toBeInTheDocument();
    expect(screen.getByText("a1")).toBeInTheDocument();
  });

  it("shows the formatted token cost", () => {
    render(<Card task={task()} tokens={47_000} />);
    expect(screen.getByText(/47k/)).toBeInTheDocument();
  });

  it("fires onClick with the task id", () => {
    const onClick = vi.fn();
    render(<Card task={task()} tokens={0} onClick={onClick} />);
    fireEvent.click(screen.getByText("Patient Records bulk-write"));
    expect(onClick).toHaveBeenCalledWith("T-42");
  });

  it("gives a gated card the accent border", () => {
    const { container } = render(<Card task={task({ state: "gated" })} tokens={0} />);
    const root = container.firstElementChild as HTMLElement;
    expect(root.style.borderColor).toContain("--accent-bd");
  });

  it("marks a revising card with the bounce-back glyph", () => {
    render(<Card task={task({ state: "revising" })} tokens={0} />);
    expect(screen.getByText(/↩/)).toBeInTheDocument();
  });
});
