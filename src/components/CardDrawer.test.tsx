import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const listMock = vi.fn();
const addMock = vi.fn();
const delMock = vi.fn();
vi.mock("../ipc/review", () => ({
  listComments: (...a: unknown[]) => listMock(...a),
  addComment: (...a: unknown[]) => addMock(...a),
  deleteComment: (...a: unknown[]) => delMock(...a),
}));

import { CardDrawer } from "./CardDrawer";
import type { Task } from "../ipc/runtime";

function task(over: Partial<Task> = {}): Task {
  return {
    id: "T-40", project_id: "p", pipeline: "p", topic: "Scheduling bulk-write",
    target_repo: null, target_scope: null, current_stage: "gate-2-plan",
    state: "gated", attempts: 1, parent_artifact: null, review_artifact: null,
    created_at: 0, updated_at: 0, ...over,
  };
}

describe("CardDrawer", () => {
  beforeEach(() => {
    listMock.mockReset().mockResolvedValue([]);
    addMock.mockReset();
    delMock.mockReset();
  });

  it("renders the task head + the three tabs", async () => {
    render(
      <CardDrawer
        task={task()}
        artifactMarkdown="# Plan\n\nbody"
        onApprove={() => {}}
        onRevise={() => {}}
        onReject={() => {}}
      />,
    );
    expect(screen.getByText("T-40")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /artifact/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /live log/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /review/i })).toBeInTheDocument();
    await waitFor(() => expect(listMock).toHaveBeenCalledWith("T-40"));
  });

  it("calls onApprove from the action bar", () => {
    const onApprove = vi.fn();
    render(
      <CardDrawer task={task()} artifactMarkdown="# Plan" onApprove={onApprove} onRevise={() => {}} onReject={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /^approve/i }));
    expect(onApprove).toHaveBeenCalledWith("T-40");
  });

  it("opens the revise panel and sends back with direction", () => {
    const onRevise = vi.fn();
    render(
      <CardDrawer task={task()} artifactMarkdown="# Plan" onApprove={() => {}} onRevise={onRevise} onReject={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /^revise/i }));
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "do over" } });
    fireEvent.click(screen.getByRole("button", { name: /send back/i }));
    expect(onRevise).toHaveBeenCalledWith("T-40", "do over");
  });

  it("calls onReject from the action bar", () => {
    const onReject = vi.fn();
    render(
      <CardDrawer task={task()} artifactMarkdown="# Plan" onApprove={() => {}} onRevise={() => {}} onReject={onReject} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /^reject/i }));
    expect(onReject).toHaveBeenCalledWith("T-40");
  });

  it("switches to the live log tab", () => {
    render(
      <CardDrawer task={task({ review_artifact: null })} artifactMarkdown="# Plan" onApprove={() => {}} onRevise={() => {}} onReject={() => {}} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /live log/i }));
    expect(screen.getByText(/no log/i)).toBeInTheDocument();
  });

  it("shows an empty artifact hint when there is no markdown", () => {
    render(
      <CardDrawer task={task()} artifactMarkdown="" onApprove={() => {}} onRevise={() => {}} onReject={() => {}} />,
    );
    // artifact tab is default; with no body the ArtifactView empty state shows.
    expect(screen.getByText(/no artifact|nothing to show|empty/i)).toBeInTheDocument();
  });

  it("renders a lineage tab that can be selected", () => {
    render(
      <CardDrawer
        task={task({ parent_artifact: "artifacts/specs/T-1-v1.md" })}
        artifactMarkdown=""
        onApprove={() => {}}
        onRevise={() => {}}
        onReject={() => {}}
      />,
    );
    const lineage = screen.getByRole("tab", { name: /lineage/i });
    expect(lineage).toBeInTheDocument();
    fireEvent.click(lineage);
    expect(screen.getByText("topic")).toBeInTheDocument();
  });
});
