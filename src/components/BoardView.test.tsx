import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { BoardView, cardLabel } from "./BoardView";
import type { Pipeline } from "../ipc/pipeline";
import type { Task } from "../ipc/runtime";

function pipeline(): Pipeline {
  return {
    id: "p",
    name: "P",
    description: "",
    schema_version: 1,
    teams: [
      { id: "research", name: "Research", workers: { min: 1, max: 2 } } as any,
      { id: "spec-writers", name: "Spec Writers", workers: { min: 1, max: 3 } } as any,
    ],
    gates: [{ id: "gate-1-spec", label: "Gate 1", downstream: "spec-writers" }],
    escalations: [],
    forks: [],
    joins: [],
  };
}

function task(id: string, stage: string, state: Task["state"], extra: Partial<Task> = {}): Task {
  return {
    id, project_id: "p", pipeline: "p", topic: `T ${id}`,
    target_repo: null, target_scope: null, current_stage: stage,
    state, attempts: 1, parent_artifact: null, review_artifact: null,
    created_at: 0, updated_at: 0, run_id: null, item_key: null, ...extra,
  };
}

describe("cardLabel", () => {
  it("labels a card by its description (topic), falling back to the slug then id", () => {
    expect(cardLabel({ topic: "A clear title", item_key: "the-slug", id: "T-1" } as Task)).toBe("A clear title");
    expect(cardLabel({ topic: "", item_key: "the-slug", id: "T-1" } as Task)).toBe("the-slug");
    expect(cardLabel({ topic: "", item_key: "", id: "T-1" } as Task)).toBe("T-1");
  });
});

describe("BoardView", () => {
  it("renders a lane per node with its label", () => {
    render(<BoardView pipeline={pipeline()} tasks={[]} hasRun onOpenCard={() => {}} />);
    expect(screen.getByText("Research")).toBeInTheDocument();
    expect(screen.getByText("Spec Writers")).toBeInTheDocument();
    expect(screen.getByText("Gate 1")).toBeInTheDocument();
  });

  it("renders a card in the lane matching its stage", () => {
    render(
      <BoardView
        pipeline={pipeline()}
        tasks={[task("T-1", "research", "running")]}
        hasRun
        onOpenCard={() => {}}
      />,
    );
    // the card leads with its description (topic), not the raw id
    expect(screen.getByText("T T-1")).toBeInTheDocument();
  });

  it("calls onOpenCard when a card is clicked", () => {
    const onOpen = vi.fn();
    render(
      <BoardView
        pipeline={pipeline()}
        tasks={[task("T-1", "research", "running")]}
        hasRun
        onOpenCard={onOpen}
      />,
    );
    fireEvent.click(screen.getByText("T T-1"));
    expect(onOpen).toHaveBeenCalledWith("T-1");
  });

  it("shows a transient clickable generator card in the source lane while active", () => {
    const onOpenCard = vi.fn();
    render(
      <BoardView
        pipeline={pipeline()}
        tasks={[]}
        hasRun
        onOpenCard={onOpenCard}
        activeGenerators={[{ stage: "research", task_id: "gen:R-1:research" }]}
      />,
    );
    const card = screen.getByText(/scanning/i);
    expect(card).toBeInTheDocument();
    fireEvent.click(card);
    expect(onOpenCard).toHaveBeenCalledWith("gen:R-1:research");
  });

  it("the generator card is keyboard-activatable and labelled (a11y, M3)", () => {
    const onOpenCard = vi.fn();
    render(
      <BoardView
        pipeline={pipeline()}
        tasks={[]}
        hasRun
        onOpenCard={onOpenCard}
        activeGenerators={[{ stage: "research", task_id: "gen:R-1:research" }]}
      />,
    );
    const card = screen.getByRole("button", { name: /generator research scanning/i });
    expect(card).toHaveAttribute("tabindex", "0");
    fireEvent.keyDown(card, { key: "Enter" });
    expect(onOpenCard).toHaveBeenCalledWith("gen:R-1:research");
  });

  it("shows an empty hint when there is no pipeline", () => {
    render(<BoardView pipeline={null} tasks={[]} onOpenCard={() => {}} />);
    expect(screen.getByText(/no active pipeline/i)).toBeInTheDocument();
  });

  it("shows the lane header when the pipeline has lanes but no tasks", () => {
    render(<BoardView pipeline={pipeline()} tasks={[]} tokensByTask={{}} hasRun onOpenCard={() => {}} />);
    // the board is not blank-blank — at minimum the lane label shows.
    expect(screen.getByText("Research")).toBeInTheDocument();
  });

  it("labels a card by its item_key, falling back to topic", () => {
    render(
      <BoardView
        pipeline={pipeline()}
        tasks={[
          task("T-1", "research", "running", { item_key: "feature/login" }),
          task("T-2", "research", "queued"),
        ]}
        hasRun
        onOpenCard={() => {}}
      />,
    );
    expect(screen.getByText("feature/login")).toBeInTheDocument();
    // T-2 has no item_key, falls back to its topic
    expect(screen.getByText("T T-2")).toBeInTheDocument();
  });

  it("shows store occupancy n/cap and pool busy/max on team lanes", () => {
    render(
      <BoardView
        pipeline={pipeline()}
        tasks={[
          task("T-1", "spec-writers", "running"),
          task("T-2", "spec-writers", "running"),
          task("T-3", "spec-writers", "queued"),
        ]}
        occupancy={[
          { stage: "research", occupancy: 0, capacity: 5 },
          { stage: "spec-writers", occupancy: 2, capacity: 3 },
        ]}
        hasRun
        onOpenCard={() => {}}
      />,
    );
    // store occupancy reads from the occupancy prop
    expect(screen.getByText("store 2/3")).toBeInTheDocument();
    expect(screen.getByText("store 0/5")).toBeInTheDocument();
    // pool busy = running tasks at that stage (2), max from Workers.max (3)
    expect(screen.getByText("pool 2/3")).toBeInTheDocument();
  });

  it("shows a run-scoped empty hint when no run is scoped", () => {
    render(<BoardView pipeline={pipeline()} tasks={[]} hasRun={false} onOpenCard={() => {}} />);
    expect(screen.getByText(/no run scoped/i)).toBeInTheDocument();
  });
});
