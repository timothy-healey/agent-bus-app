import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { NewProjectWizard } from "./NewProjectWizard";
import { emptyDraft } from "./draft";

const kickoffMock = vi.fn();
const listSeedTemplatesMock = vi.fn();
const seedTemplateMock = vi.fn();
vi.mock("../ipc/pipeline", () => ({
  kickoffGenerate: (...a: unknown[]) => kickoffMock(...a),
  listSeedTemplates: (...a: unknown[]) => listSeedTemplatesMock(...a),
  seedTemplate: (...a: unknown[]) => seedTemplateMock(...a),
  // ChatDraftPanel / ReviewStep import these too; stub them so the shell test is isolated.
  designSessionTurn: vi.fn(),
  createProjectFromDraft: vi.fn(),
}));

describe("NewProjectWizard", () => {
  beforeEach(() => {
    kickoffMock.mockReset();
    seedTemplateMock.mockReset();
    // default: no templates (the picker is hidden) unless a test opts in.
    listSeedTemplatesMock.mockReset();
    listSeedTemplatesMock.mockResolvedValue([]);
  });

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

  it("starting from a template seeds the draft and advances to Teams", async () => {
    const seeded = {
      ...emptyDraft(),
      id: "ddd-spec-plan-impl",
      teams: [{ id: "research", name: "Research", prompt_body: "x", runner: { kind: "claude-cli", model: "m", effort: { mode: "standard" } }, scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { default: 1, max: 1 } }],
    };
    listSeedTemplatesMock.mockResolvedValue([{ id: "ddd-spec-plan-impl", name: "DDD Spec → Plan → Implement", description: "d" }]);
    seedTemplateMock.mockResolvedValueOnce(seeded);
    render(<NewProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "Demo" } });
    fireEvent.change(screen.getByLabelText(/root path/i), { target: { value: "/p" } });
    // the picker button appears once templates load
    const btn = await screen.findByRole("button", { name: /DDD Spec/ });
    fireEvent.click(btn);
    await waitFor(() => expect(seedTemplateMock).toHaveBeenCalledWith("ddd-spec-plan-impl"));
    // advanced to the Teams step
    expect(await screen.findByRole("heading", { name: /teams/i })).toBeInTheDocument();
  });

  it("Cancel calls onClose", () => {
    const onClose = vi.fn();
    render(<NewProjectWizard open={true} onClose={onClose} onCreated={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(onClose).toHaveBeenCalled();
  });
});
