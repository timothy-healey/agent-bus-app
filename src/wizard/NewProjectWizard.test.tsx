import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { NewProjectWizard } from "./NewProjectWizard";
import { emptyDraft } from "./draft";

const kickoffMock = vi.fn();
const listSeedTemplatesMock = vi.fn();
const seedTemplateMock = vi.fn();
const createMock = vi.fn();
vi.mock("../ipc/pipeline", () => ({
  kickoffGenerate: (...a: unknown[]) => kickoffMock(...a),
  listSeedTemplates: (...a: unknown[]) => listSeedTemplatesMock(...a),
  seedTemplate: (...a: unknown[]) => seedTemplateMock(...a),
  createProjectFromDraft: (...a: unknown[]) => createMock(...a),
  // ChatDraftPanel imports this too; stub it so the shell test is isolated.
  designSessionTurn: vi.fn(),
}));

// A draft with one team, enough to reach + render the review step.
function draftWithTeam() {
  return { ...emptyDraft(), teams: [{ id: "research", name: "Research", prompt_body: "", runner: { kind: "claude-cli", model: "m", effort: { mode: "standard" } }, scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { default: 1, max: 1 } }] };
}

// Open the wizard, fill basics, generate, and advance through to the review step.
async function openToReview() {
  kickoffMock.mockResolvedValueOnce(draftWithTeam());
  const onCreated = vi.fn();
  render(<NewProjectWizard open={true} onClose={() => {}} onCreated={onCreated} />);
  fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "Demo" } });
  fireEvent.change(screen.getByLabelText(/root path/i), { target: { value: "/p" } });
  fireEvent.change(screen.getByLabelText(/describe/i), { target: { value: "a research flow" } });
  fireEvent.click(screen.getByRole("button", { name: /generate/i }));
  await screen.findByRole("heading", { name: /teams/i });
  fireEvent.click(screen.getByRole("button", { name: /next/i })); // teams -> prompts
  fireEvent.click(screen.getByRole("button", { name: /next/i })); // prompts -> wiring
  fireEvent.click(screen.getByRole("button", { name: /next/i })); // wiring -> review
  return { onCreated };
}

describe("NewProjectWizard", () => {
  beforeEach(() => {
    kickoffMock.mockReset();
    seedTemplateMock.mockReset();
    createMock.mockReset();
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

  it("the review step shows the footer Create + Back actions", async () => {
    await openToReview();
    expect(screen.getByText(/prompts\/research\.md/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /create project/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /back/i })).toBeInTheDocument();
    // No Next on review.
    expect(screen.queryByRole("button", { name: /next/i })).not.toBeInTheDocument();
  });

  it("footer Create calls createProjectFromDraft and onCreated on success", async () => {
    const created = { id: "proj-x", name: "Demo", root_path: "/p", active_pipeline_id: "demo", created_at: 0, updated_at: 0 };
    createMock.mockResolvedValueOnce(created);
    const { onCreated } = await openToReview();
    fireEvent.click(screen.getByRole("button", { name: /create project/i }));
    await waitFor(() => expect(createMock).toHaveBeenCalledWith("Demo", "/p", expect.objectContaining({ name: "Demo" })));
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith(created));
  });

  it("footer Create surfaces the backend error when create fails (writes nothing)", async () => {
    createMock.mockRejectedValueOnce(new Error("team 'research' is unreachable"));
    await openToReview();
    fireEvent.click(screen.getByRole("button", { name: /create project/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/unreachable/);
  });
});
