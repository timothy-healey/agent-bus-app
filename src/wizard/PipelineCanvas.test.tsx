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

  it("does NOT show the prominent banner from the start (subtle badges only) — G11", async () => {
    const { bestEffortValidate } = await import("../ipc/pipeline");
    (bestEffortValidate as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(["team 'a' has no prompt yet"]);
    render(<PipelineCanvas draft={addTeam(emptyDraft(), "a", "A")} onChange={() => {}} />);
    // give the async validation a tick
    await waitFor(() => expect(bestEffortValidate).toHaveBeenCalled());
    expect(screen.queryByRole("alert", { name: /validation issues/i })).not.toBeInTheDocument();
  });

  it("shows the prominent banner only when showBanner is set — G11", async () => {
    const { bestEffortValidate } = await import("../ipc/pipeline");
    (bestEffortValidate as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(["team 'a' has no prompt yet"]);
    render(<PipelineCanvas draft={addTeam(emptyDraft(), "a", "A")} onChange={() => {}} showBanner />);
    await waitFor(() => expect(screen.getByRole("alert", { name: /validation issues/i })).toHaveTextContent(/no prompt yet/));
  });

  it("reports validity to the host via onValidityChange — G11", async () => {
    const { bestEffortValidate } = await import("../ipc/pipeline");
    (bestEffortValidate as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce(["team 'a' has no prompt yet"]);
    const onValidityChange = vi.fn();
    render(<PipelineCanvas draft={addTeam(emptyDraft(), "a", "A")} onChange={() => {}} onValidityChange={onValidityChange} />);
    await waitFor(() => expect(onValidityChange).toHaveBeenCalledWith(false, ["team 'a' has no prompt yet"]));
  });
});

describe("PipelineCanvas — node delete (G9)", () => {
  it("right-click opens a context menu with Delete, which removes the node", async () => {
    let draft = addTeam(emptyDraft(), "research", "Research");
    const onChange = (d: DraftPipeline) => { draft = d; rerender(<PipelineCanvas draft={draft} onChange={onChange} />); };
    const { rerender, container } = render(<PipelineCanvas draft={draft} onChange={onChange} />);
    // The custom node renders the team's aria-label; right-click it.
    const node = await screen.findByLabelText(/team research/i);
    fireEvent.contextMenu(node);
    const item = await screen.findByRole("menuitem", { name: /delete node/i });
    fireEvent.click(item);
    expect(draft.teams).toHaveLength(0);
    expect(container).toBeTruthy();
  });

  it("Delete key removes the selected node (and is guarded inside text inputs)", async () => {
    let draft = addTeam(emptyDraft(), "research", "Research");
    const onChange = (d: DraftPipeline) => { draft = d; rerender(<PipelineCanvas draft={draft} onChange={onChange} />); };
    const { rerender } = render(<PipelineCanvas draft={draft} onChange={onChange} />);
    // select the node (opens its drawer)
    const node = await screen.findByLabelText(/team research/i);
    fireEvent.click(node);
    // typing Delete while focused in the drawer's name field must NOT delete
    const nameField = screen.getByLabelText("name for research");
    nameField.focus();
    fireEvent.keyDown(screen.getByTestId("pipeline-canvas"), { key: "Delete" });
    expect(draft.teams).toHaveLength(1);
    // blur the field, then Delete on the canvas removes the selected node
    nameField.blur();
    fireEvent.keyDown(screen.getByTestId("pipeline-canvas"), { key: "Delete" });
    expect(draft.teams).toHaveLength(0);
  });

  it("the NodeDrawer header Delete button removes the node", async () => {
    let draft = addTeam(emptyDraft(), "research", "Research");
    const onChange = (d: DraftPipeline) => { draft = d; rerender(<PipelineCanvas draft={draft} onChange={onChange} />); };
    const { rerender } = render(<PipelineCanvas draft={draft} onChange={onChange} />);
    fireEvent.click(await screen.findByLabelText(/team research/i));
    fireEvent.click(screen.getByRole("button", { name: /delete research/i }));
    expect(draft.teams).toHaveLength(0);
  });
});
