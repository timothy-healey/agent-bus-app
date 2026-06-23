import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LineageTab } from "./LineageTab";
import type { Task } from "../ipc/runtime";

function t(over: Partial<Task>): Task {
  return {
    id: "T-1", project_id: "p", pipeline: "pipe", topic: "Bulk write",
    target_repo: null, target_scope: null, current_stage: "plan-writers",
    state: "gated", attempts: 1, parent_artifact: null, review_artifact: null,
    created_at: 0, updated_at: 0, ...over,
  };
}

describe("LineageTab", () => {
  it("renders the topic root and each artifact entry", () => {
    render(<LineageTab task={t({ parent_artifact: "artifacts/specs/T-1-v1.md" })} onOpenArtifact={() => {}} />);
    expect(screen.getByText("topic")).toBeInTheDocument();
    expect(screen.getByText("T-1-v1.md")).toBeInTheDocument();
  });

  it("calls onOpenArtifact with the path when an entry is clicked", () => {
    const onOpen = vi.fn();
    render(<LineageTab task={t({ parent_artifact: "artifacts/specs/T-1-v1.md" })} onOpenArtifact={onOpen} />);
    fireEvent.click(screen.getByText("T-1-v1.md"));
    expect(onOpen).toHaveBeenCalledWith("artifacts/specs/T-1-v1.md");
  });

  it("shows only the topic when the task has no artifacts yet", () => {
    render(<LineageTab task={t({})} onOpenArtifact={() => {}} />);
    expect(screen.getByText("topic")).toBeInTheDocument();
    expect(screen.getByText(/no upstream artifacts/i)).toBeInTheDocument();
  });

  it("enters compare mode and calls onCompare once two entries are selected", () => {
    const onCompare = vi.fn();
    render(
      <LineageTab
        task={t({ parent_artifact: "artifacts/specs/T-1-v1.md", review_artifact: "artifacts/reviews/T-1-rev.md" })}
        onOpenArtifact={() => {}}
        onCompare={onCompare}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /^compare/i }));
    // In compare mode each artifact entry is a selectable checkbox.
    fireEvent.click(screen.getByRole("checkbox", { name: /T-1-v1\.md/ }));
    expect(onCompare).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("checkbox", { name: /T-1-rev\.md/ }));
    expect(onCompare).toHaveBeenCalledWith("artifacts/specs/T-1-v1.md", "artifacts/reviews/T-1-rev.md");
  });

  it("does not offer compare when fewer than two artifacts exist", () => {
    render(
      <LineageTab
        task={t({ parent_artifact: "artifacts/specs/T-1-v1.md" })}
        onOpenArtifact={() => {}}
        onCompare={() => {}}
      />,
    );
    expect(screen.queryByRole("button", { name: /^compare/i })).not.toBeInTheDocument();
  });
});
