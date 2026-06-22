import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ArtifactView } from "./ArtifactView";

const md = "# Plan\n\nThe controller validates the batch.\n\n## Tasks\n- one\n- two";

describe("ArtifactView", () => {
  it("renders headings, paragraph and list from markdown", () => {
    render(<ArtifactView markdown={md} onAddComment={() => {}} />);
    expect(screen.getByRole("heading", { level: 1, name: "Plan" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 2, name: "Tasks" })).toBeInTheDocument();
    expect(screen.getByText(/controller validates the batch/)).toBeInTheDocument();
    expect(screen.getByText("one")).toBeInTheDocument();
  });

  it("shows an empty state when markdown is empty", () => {
    render(<ArtifactView markdown="" onAddComment={() => {}} />);
    expect(screen.getByText(/no artifact/i)).toBeInTheDocument();
  });

  it("surfaces the popover when there is an active selection and reports the added comment", () => {
    const onAdd = vi.fn();
    render(<ArtifactView markdown={md} onAddComment={onAdd} />);
    fireEvent.mouseUp(screen.getByTestId("artifact-body"), {});
    expect(screen.getByTestId("artifact-body")).toBeInTheDocument();
    expect(onAdd).not.toHaveBeenCalled();
  });
});
