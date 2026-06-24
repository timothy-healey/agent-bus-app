import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { PipelineCanvas } from "./PipelineCanvas";
import { emptyDraft, addTeam } from "./draft";
import type { DraftPipeline } from "../ipc/pipeline";

vi.mock("../ipc/pipeline", async () => {
  const actual = await vi.importActual<typeof import("../ipc/pipeline")>("../ipc/pipeline");
  return { ...actual, bestEffortValidate: vi.fn().mockResolvedValue([]) };
});

describe("PipelineCanvas", () => {
  it("renders the NodeKind-driven palette", () => {
    render(<PipelineCanvas draft={emptyDraft()} onChange={() => {}} />);
    for (const k of ["team", "gate", "fork", "join", "escalation"]) {
      expect(screen.getByRole("button", { name: new RegExp(`add ${k}`, "i") })).toBeInTheDocument();
    }
  });

  it("shows an empty-state for a draft with no nodes", () => {
    render(<PipelineCanvas draft={emptyDraft()} onChange={() => {}} />);
    expect(screen.getByText(/empty pipeline/i)).toBeInTheDocument();
  });

  it("palette add → team added to the draft via onChange", () => {
    const onChange = vi.fn();
    render(<PipelineCanvas draft={emptyDraft()} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /add team/i }));
    const next: DraftPipeline = onChange.mock.calls.at(-1)![0];
    expect(next.teams).toHaveLength(1);
  });

  it("selecting the newly-added node opens its drawer (controlled re-render)", () => {
    // Simulate the controlled cycle: parent holds the draft.
    let draft = emptyDraft();
    const onChange = (d: DraftPipeline) => { draft = d; rerender(<PipelineCanvas draft={draft} onChange={onChange} />); };
    const { rerender } = render(<PipelineCanvas draft={draft} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /add team/i }));
    // the new team's drawer is open + selected
    expect(screen.getByRole("dialog", { name: /team · team-1/i })).toBeInTheDocument();
  });

  it("surfaces best-effort validation issues in the amber banner", async () => {
    const { bestEffortValidate } = await import("../ipc/pipeline");
    (bestEffortValidate as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(["team 'a' has no prompt yet"]);
    render(<PipelineCanvas draft={addTeam(emptyDraft(), "a", "A")} onChange={() => {}} />);
    await waitFor(() => expect(screen.getByRole("status", { name: /validation issues/i })).toHaveTextContent(/no prompt yet/));
  });
});
