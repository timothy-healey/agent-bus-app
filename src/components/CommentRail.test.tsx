import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { CommentRail } from "./CommentRail";
import type { Comment } from "../ipc/review";

function comment(over: Partial<Comment> = {}): Comment {
  return {
    id: "c1",
    task_id: "T-1",
    artifact_path: "a.md",
    anchor_text: "the span",
    anchor_offset: 10,
    note: "per-row, not per-batch",
    kind: "inline",
    created_at: 0,
    ...over,
  };
}

describe("CommentRail", () => {
  it("shows an empty state when there are no comments", () => {
    render(<CommentRail comments={[]} onSelect={() => {}} onDelete={() => {}} />);
    expect(screen.getByText(/select text to comment/i)).toBeInTheDocument();
  });

  it("renders the count and each comment's note + quote", () => {
    render(
      <CommentRail
        comments={[comment(), comment({ id: "c2", note: "second", anchor_text: "other" })]}
        onSelect={() => {}}
        onDelete={() => {}}
      />,
    );
    expect(screen.getByTestId("rail-count")).toHaveTextContent("2");
    expect(screen.getByText("per-row, not per-batch")).toBeInTheDocument();
    expect(screen.getByText(/the span/)).toBeInTheDocument();
    expect(screen.getByText("second")).toBeInTheDocument();
  });

  it("numbers inline comments in order", () => {
    render(
      <CommentRail
        comments={[comment(), comment({ id: "c2" })]}
        onSelect={() => {}}
        onDelete={() => {}}
      />,
    );
    expect(screen.getByText("1")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
  });

  it("calls onSelect when an entry is clicked", () => {
    const onSelect = vi.fn();
    render(<CommentRail comments={[comment()]} onSelect={onSelect} onDelete={() => {}} />);
    fireEvent.click(screen.getByText("per-row, not per-batch"));
    expect(onSelect).toHaveBeenCalledWith("c1");
  });

  it("calls onDelete when the delete affordance is clicked", () => {
    const onDelete = vi.fn();
    render(<CommentRail comments={[comment()]} onSelect={() => {}} onDelete={onDelete} />);
    fireEvent.click(screen.getByRole("button", { name: /delete comment/i }));
    expect(onDelete).toHaveBeenCalledWith("c1");
  });
});
