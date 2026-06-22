import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { BoardView } from "./BoardView";
import type { Pipeline } from "../ipc/pipeline";
import type { Task } from "../ipc/runtime";

function pipeline(): Pipeline {
  return {
    id: "p",
    name: "P",
    description: "",
    schema_version: 1,
    teams: [
      { id: "research", name: "Research" } as any,
      { id: "spec-writers", name: "Spec Writers" } as any,
    ],
    gates: [{ id: "gate-1-spec", label: "Gate 1", downstream: "spec-writers" }],
    escalations: [],
  };
}

function task(id: string, stage: string, state: Task["state"]): Task {
  return {
    id, project_id: "p", pipeline: "p", topic: `T ${id}`,
    target_repo: null, target_scope: null, current_stage: stage,
    state, attempts: 1, parent_artifact: null, review_artifact: null,
    created_at: 0, updated_at: 0,
  };
}

describe("BoardView", () => {
  it("renders a lane per node with its label", () => {
    render(<BoardView pipeline={pipeline()} tasks={[]} onOpenCard={() => {}} />);
    expect(screen.getByText("Research")).toBeInTheDocument();
    expect(screen.getByText("Spec Writers")).toBeInTheDocument();
    expect(screen.getByText("Gate 1")).toBeInTheDocument();
  });

  it("renders a card in the lane matching its stage", () => {
    render(
      <BoardView
        pipeline={pipeline()}
        tasks={[task("T-1", "research", "running")]}
        onOpenCard={() => {}}
      />,
    );
    expect(screen.getByText("T-1")).toBeInTheDocument();
  });

  it("calls onOpenCard when a card is clicked", () => {
    const onOpen = vi.fn();
    render(
      <BoardView
        pipeline={pipeline()}
        tasks={[task("T-1", "research", "running")]}
        onOpenCard={onOpen}
      />,
    );
    fireEvent.click(screen.getByText("T-1"));
    expect(onOpen).toHaveBeenCalledWith("T-1");
  });

  it("shows an empty hint when there is no pipeline", () => {
    render(<BoardView pipeline={null} tasks={[]} onOpenCard={() => {}} />);
    expect(screen.getByText(/no active pipeline/i)).toBeInTheDocument();
  });

  it("shows the lane header when the pipeline has lanes but no tasks", () => {
    render(<BoardView pipeline={pipeline()} tasks={[]} tokensByTask={{}} onOpenCard={() => {}} />);
    // the board is not blank-blank — at minimum the lane label shows.
    expect(screen.getByText("Research")).toBeInTheDocument();
  });
});
