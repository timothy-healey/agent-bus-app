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
  // PipelineCanvas runs live best-effort validation; stub it so the shell test
  // is isolated.
  bestEffortValidate: vi.fn().mockResolvedValue([]),
}));

// A draft with one team, enough to reach + render the review step.
function draftWithTeam() {
  return { ...emptyDraft(), teams: [{ id: "research", name: "Research", prompt_body: "", runner: { kind: "claude-cli", model: "m", effort: { mode: "standard" } }, scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { min: 1, max: 1 } }] };
}

// Open the wizard, fill basics, generate, and advance through to the review step.
async function openToReview(targetRepo?: string) {
  kickoffMock.mockResolvedValueOnce(draftWithTeam());
  const onCreated = vi.fn();
  render(<NewProjectWizard open={true} onClose={() => {}} onCreated={onCreated} />);
  fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "Demo" } });
  fireEvent.change(screen.getByRole("textbox", { name: "Root path" }), { target: { value: "/p" } });
  if (targetRepo !== undefined) {
    fireEvent.change(screen.getByRole("textbox", { name: /target repo/i }), { target: { value: targetRepo } });
  }
  fireEvent.change(screen.getByLabelText(/describe/i), { target: { value: "a research flow" } });
  fireEvent.click(screen.getByRole("button", { name: /generate/i }));
  // generate advances to the Canvas step (its palette is the tell)
  await screen.findByRole("button", { name: /add team/i });
  fireEvent.click(screen.getByRole("button", { name: /continue/i })); // canvas -> review
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
    const draft = { ...emptyDraft(), teams: [{ id: "research", name: "Research", prompt_body: "", runner: { kind: "claude-cli", model: "m", effort: { mode: "standard" } }, scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { min: 1, max: 1 } }] };
    kickoffMock.mockResolvedValueOnce(draft);
    render(<NewProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "Demo" } });
    fireEvent.change(screen.getByRole("textbox", { name: "Root path" }), { target: { value: "/p" } });
    fireEvent.change(screen.getByLabelText(/describe/i), { target: { value: "a research flow" } });
    fireEvent.click(screen.getByRole("button", { name: /generate/i }));
    await waitFor(() => expect(kickoffMock).toHaveBeenCalled());
    // advanced to the Canvas step (its palette is visible)
    expect(await screen.findByRole("button", { name: /add team/i })).toBeInTheDocument();
  });

  it("starting from a template seeds the draft and advances to Teams", async () => {
    const seeded = {
      ...emptyDraft(),
      id: "ddd-spec-plan-impl",
      teams: [{ id: "research", name: "Research", prompt_body: "x", runner: { kind: "claude-cli", model: "m", effort: { mode: "standard" } }, scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { min: 1, max: 1 } }],
    };
    listSeedTemplatesMock.mockResolvedValue([{ id: "ddd-spec-plan-impl", name: "DDD Spec → Plan → Implement", description: "d" }]);
    seedTemplateMock.mockResolvedValueOnce(seeded);
    render(<NewProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "Demo" } });
    fireEvent.change(screen.getByRole("textbox", { name: "Root path" }), { target: { value: "/p" } });
    // the picker button appears once templates load
    const btn = await screen.findByRole("button", { name: /DDD Spec/ });
    fireEvent.click(btn);
    await waitFor(() => expect(seedTemplateMock).toHaveBeenCalledWith("ddd-spec-plan-impl"));
    // advanced to the Canvas step (its palette is visible)
    expect(await screen.findByRole("button", { name: /add team/i })).toBeInTheDocument();
  });

  it("the left-nav tree jumps directly to a step (G14)", async () => {
    await openToReview();
    // On review now; jump straight back to Canvas via the nav tree (not Back).
    fireEvent.click(screen.getByRole("button", { name: "Canvas" }));
    expect(await screen.findByRole("button", { name: /add team/i })).toBeInTheDocument();
    // and jump straight to Review again.
    fireEvent.click(screen.getByRole("button", { name: "Review" }));
    expect(await screen.findByText(/prompts\/research\.md/)).toBeInTheDocument();
  });

  it("Cancel calls onClose", () => {
    const onClose = vi.fn();
    render(<NewProjectWizard open={true} onClose={onClose} onCreated={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(onClose).toHaveBeenCalled();
  });

  it("surfaces the project switcher with delete in the left nav (G13/G14)", async () => {
    const onDeleteProject = vi.fn().mockResolvedValue(undefined);
    const projects = [
      { id: "p1", name: "Alpha", root_path: "/p1", target_repo: null, skill_sources: [], active_pipeline_id: null, created_at: 0, updated_at: 0 },
    ];
    render(
      <NewProjectWizard
        open={true}
        onClose={() => {}}
        onCreated={() => {}}
        projects={projects}
        activeProjectId="p1"
        onSelectProject={() => {}}
        onDeleteProject={onDeleteProject}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /delete project alpha/i }));
    fireEvent.click(screen.getByRole("button", { name: /confirm delete alpha/i }));
    await waitFor(() => expect(onDeleteProject).toHaveBeenCalledWith("p1"));
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
    await waitFor(() => expect(createMock).toHaveBeenCalledWith("Demo", "/p", expect.objectContaining({ name: "Demo" }), null));
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith(created));
  });

  it("footer Create passes the typed Target repo through (A5)", async () => {
    const created = { id: "proj-x", name: "Demo", root_path: "/p", target_repo: "~/repo", active_pipeline_id: "demo", created_at: 0, updated_at: 0 };
    createMock.mockResolvedValueOnce(created);
    await openToReview("~/repo");
    fireEvent.click(screen.getByRole("button", { name: /create project/i }));
    await waitFor(() => expect(createMock).toHaveBeenCalledWith("Demo", "/p", expect.objectContaining({ name: "Demo" }), "~/repo"));
  });

  it("footer Create surfaces the backend error when create fails (writes nothing)", async () => {
    createMock.mockRejectedValueOnce(new Error("team 'research' is unreachable"));
    await openToReview();
    fireEvent.click(screen.getByRole("button", { name: /create project/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/unreachable/);
  });
});
