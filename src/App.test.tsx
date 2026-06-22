import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

// Mock the data hooks/IPC the board path uses.
const tasks = [
  {
    id: "T-40", project_id: "p", pipeline: "p", topic: "Scheduling bulk-write",
    target_repo: null, target_scope: null, current_stage: "gate-2-plan",
    state: "gated", attempts: 1, parent_artifact: null, review_artifact: null,
    created_at: 0, updated_at: 0,
  },
];

vi.mock("./hooks/useTasks", () => ({
  useTasks: () => ({ tasks, loading: false, reload: vi.fn() }),
}));
vi.mock("./hooks/useProjects", () => ({
  useProjects: () => ({
    projects: [{ id: "p", name: "Proj", root_path: "/p", active_pipeline_id: "pl", created_at: 0, updated_at: 0 }],
    loading: false,
    reload: vi.fn(),
  }),
}));

const loadPipelineMock = vi.fn();
const instantiateMock = vi.fn();
vi.mock("./ipc/pipeline", () => ({
  loadPipeline: (...a: unknown[]) => loadPipelineMock(...a),
  instantiateTemplate: (...a: unknown[]) => instantiateMock(...a),
  listPipelines: vi.fn().mockResolvedValue([]),
  listTemplates: vi.fn().mockResolvedValue([]),
}));

const approveMock = vi.fn();
vi.mock("./ipc/runtime", async (orig) => {
  const actual = await (orig as any)();
  return { ...actual, approveGate: (...a: unknown[]) => approveMock(...a) };
});
vi.mock("./ipc/review", () => ({
  recordVerdict: vi.fn().mockResolvedValue({ task_id: "T-40", verdict: "approve", comment_count: 0 }),
  listComments: vi.fn().mockResolvedValue([]),
  addComment: vi.fn(),
  deleteComment: vi.fn(),
}));

import App from "./App";

describe("App board integration", () => {
  beforeEach(() => {
    approveMock.mockReset();
    const pl = {
      id: "pl", name: "PL", description: "", schema_version: 1,
      teams: [{ id: "plan-writers", name: "Plan Writers" }],
      gates: [{ id: "gate-2-plan", label: "Gate 2", downstream: "implementers" }],
      escalations: [],
    };
    loadPipelineMock.mockReset().mockResolvedValue(pl);
    // listPipelines is mocked to [] so App falls through to instantiateTemplate.
    instantiateMock.mockReset().mockResolvedValue(pl);
  });

  it("renders the board with the gated card and opens the drawer on click", async () => {
    render(<App />);
    await waitFor(() => expect(screen.getByText("T-40")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Scheduling bulk-write"));
    // drawer head shows the id (appears again inside the drawer)
    await waitFor(() =>
      expect(screen.getAllByText("T-40").length).toBeGreaterThan(1),
    );
  });
});
