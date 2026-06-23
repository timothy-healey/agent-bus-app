import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { NewProjectWizard } from "./NewProjectWizard";
import { emptyDraft } from "./draft";

const kickoffMock = vi.fn();
vi.mock("../ipc/pipeline", () => ({
  kickoffGenerate: (...a: unknown[]) => kickoffMock(...a),
  // ChatDraftPanel / ReviewStep import these too; stub them so the shell test is isolated.
  designSessionTurn: vi.fn(),
  createProjectFromDraft: vi.fn(),
}));

describe("NewProjectWizard", () => {
  beforeEach(() => kickoffMock.mockReset());

  it("does not render when open=false", () => {
    render(<NewProjectWizard open={false} onClose={() => {}} onCreated={() => {}} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("starts on Basics and generates a draft, advancing to Teams", async () => {
    const draft = { ...emptyDraft(), teams: [{ id: "research", name: "Research", prompt_body: "", runner: { kind: "claude-cli", model: "m", effort: { mode: "standard" } }, scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { default: 1, max: 1 } }] };
    kickoffMock.mockResolvedValueOnce(draft);
    render(<NewProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "Demo" } });
    fireEvent.change(screen.getByLabelText(/root path/i), { target: { value: "/p" } });
    fireEvent.change(screen.getByLabelText(/describe/i), { target: { value: "a research flow" } });
    fireEvent.click(screen.getByRole("button", { name: /generate/i }));
    await waitFor(() => expect(kickoffMock).toHaveBeenCalled());
    // advanced to the Teams step (step heading visible)
    expect(await screen.findByRole("heading", { name: /teams/i })).toBeInTheDocument();
  });

  it("Cancel calls onClose", () => {
    const onClose = vi.fn();
    render(<NewProjectWizard open={true} onClose={onClose} onCreated={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(onClose).toHaveBeenCalled();
  });
});
