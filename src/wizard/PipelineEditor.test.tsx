import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { PipelineEditor } from "./PipelineEditor";
import type { DraftPipeline } from "../ipc/pipeline";

// Mock the IPC layer (no Tauri runtime in vitest).
vi.mock("../ipc/pipeline", async () => {
  const actual = await vi.importActual<typeof import("../ipc/pipeline")>("../ipc/pipeline");
  return {
    ...actual,
    bestEffortValidate: vi.fn().mockResolvedValue([]),
    savePipelineEdits: vi.fn().mockResolvedValue(undefined),
  };
});

import { savePipelineEdits } from "../ipc/pipeline";

function seedDraft(): DraftPipeline {
  return {
    id: "demo",
    name: "Demo",
    description: "",
    schema_version: 3,
    teams: [
      {
        id: "research",
        name: "Research",
        prompt_body: "investigate",
        runner: { kind: "claude-cli", model: "claude-opus-4-8", effort: { mode: "standard" }, api_key_env: null },
        scope: { reads: [], writes: [], tools: [] },
        outputs: {},
        workers: { min: 1, max: 1 },
        role: "producer",
        store: { capacity: 8 },
      },
    ],
    forks: [],
    joins: [],
    gates: [],
    escalations: [],
  };
}

describe("PipelineEditor", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders the shared canvas surface (palette) over the seeded draft", () => {
    render(<PipelineEditor projectId="p1" seed={seedDraft()} onClose={() => {}} onSaved={() => {}} />);
    expect(screen.getByRole("button", { name: /add team/i })).toBeInTheDocument();
  });

  it("saves the current draft via savePipelineEdits and calls onSaved", async () => {
    const onSaved = vi.fn();
    render(<PipelineEditor projectId="p1" seed={seedDraft()} onClose={() => {}} onSaved={onSaved} />);
    fireEvent.click(screen.getByRole("button", { name: /save pipeline/i }));
    await waitFor(() => expect(savePipelineEdits).toHaveBeenCalledTimes(1));
    const [pid, draft] = (savePipelineEdits as unknown as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(pid).toBe("p1");
    expect(draft.teams[0].name).toBe("Research");
    await waitFor(() => expect(onSaved).toHaveBeenCalled());
  });

  it("edits round-trip through the canvas into the saved draft (add a team)", async () => {
    render(<PipelineEditor projectId="p1" seed={seedDraft()} onClose={() => {}} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /add team/i }));
    fireEvent.click(screen.getByRole("button", { name: /save pipeline/i }));
    await waitFor(() => expect(savePipelineEdits).toHaveBeenCalled());
    const [, draft] = (savePipelineEdits as unknown as ReturnType<typeof vi.fn>).mock.calls.at(-1)!;
    expect(draft.teams.map((t: { id: string }) => t.id)).toContain("team-1");
  });

  it("surfaces the backend error when save fails", async () => {
    (savePipelineEdits as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error("team 'research' is unreachable"));
    render(<PipelineEditor projectId="p1" seed={seedDraft()} onClose={() => {}} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /save pipeline/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/unreachable/);
  });
});
