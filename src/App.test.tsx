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
  {
    id: "T-99", project_id: "p", pipeline: "p", topic: "Escalated item",
    target_repo: null, target_scope: null, current_stage: "needs-human",
    state: "needs_human", attempts: 3, parent_artifact: null, review_artifact: null,
    created_at: 0, updated_at: 0, run_id: "R-1",
  },
];

vi.mock("./hooks/useTasks", () => ({
  useTasks: () => ({
    tasks, loading: false, reload: vi.fn(),
    // ④e: the board scopes by run; T-40 is keyed under run R-1 so it shows on
    // the board when R-1 is the selected run.
    tasksByRun: new Map([["R-1", tasks]]),
  }),
}));

const run1 = { id: "R-1", pipeline: "pl", project_id: "p", generator_dry: false, completed: false, created_at: 0 };
vi.mock("./hooks/useRuns", () => ({
  useRuns: () => ({
    runs: [run1], loading: false, selectedRun: run1, activeRun: run1, select: vi.fn(), reload: vi.fn(),
  }),
}));
vi.mock("./hooks/useStoreOccupancy", () => ({
  useStoreOccupancy: () => ({ occupancy: [], loading: false, reload: vi.fn() }),
}));
vi.mock("./hooks/useProjects", () => ({
  useProjects: () => ({
    projects: [{ id: "p", name: "Proj", root_path: "/p", active_pipeline_id: "pl", created_at: 0, updated_at: 0 }],
    loading: false,
    reload: vi.fn(),
  }),
}));

const loadPipelineMock = vi.fn();
vi.mock("./ipc/pipeline", () => ({
  loadPipeline: (...a: unknown[]) => loadPipelineMock(...a),
  listPipelines: vi.fn().mockResolvedValue(["pl"]),
  // NewProjectWizard's import tree pulls these from ./ipc/pipeline.
  kickoffGenerate: vi.fn(),
  designSessionTurn: vi.fn(),
  createProjectFromDraft: vi.fn(),
}));

const approveMock = vi.fn();
const listInvocationsMock = vi.fn();
const retryTaskMock = vi.fn();
const acceptTaskMock = vi.fn();
vi.mock("./ipc/runtime", async (orig) => {
  const actual = await (orig as any)();
  return {
    ...actual,
    approveGate: (...a: unknown[]) => approveMock(...a),
    listInvocations: (...a: unknown[]) => listInvocationsMock(...a),
    retryTask: (...a: unknown[]) => retryTaskMock(...a),
    acceptTask: (...a: unknown[]) => acceptTaskMock(...a),
  };
});
vi.mock("./ipc/review", () => ({
  recordVerdict: vi.fn().mockResolvedValue({ task_id: "T-40", verdict: "approve", comment_count: 0 }),
  listComments: vi.fn().mockResolvedValue([]),
  reanchorComments: vi.fn().mockResolvedValue([]),
  addComment: vi.fn(),
  deleteComment: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue({ on: false, reason: null }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("./hooks/useUsage", () => ({ useUsage: () => ({ snapshot: null, reload: vi.fn() }) }));
vi.mock("./ipc/terminal", () => ({
  getConversation: vi.fn().mockResolvedValue(null),
  sendMessage: vi.fn().mockResolvedValue({
    project_id: "p", session_id: "s", started_at: 0, last_message_at: 0,
    turns: [], summary_of_prior_sessions: null, history_budget_tokens: 8000,
  }),
  onConversationDelta: vi.fn().mockResolvedValue(() => {}),
}));

import App from "./App";

describe("App board integration", () => {
  beforeEach(() => {
    approveMock.mockReset();
    listInvocationsMock.mockReset().mockResolvedValue([
      { invocation_id: "I-1", team_id: "implementers", model: "m", attempts: 3, started_at: 0, settled_at: 1, outcome: "error:rate_limited", input_tokens: 0, output_tokens: 0 },
    ]);
    retryTaskMock.mockReset().mockResolvedValue({ id: "T-99", state: "queued" });
    acceptTaskMock.mockReset().mockResolvedValue({ id: "T-99", state: "done" });
    const pl = {
      id: "pl", name: "PL", description: "", schema_version: 2,
      teams: [{ id: "plan-writers", name: "Plan Writers" }],
      gates: [{ id: "gate-2-plan", label: "Gate 2", downstream: "implementers" }],
      escalations: [{ id: "needs-human", triggers: [] }], forks: [], joins: [],
    };
    loadPipelineMock.mockReset().mockResolvedValue(pl);
  });

  it("renders the board with the gated card and opens the drawer on click", async () => {
    render(<App />);
    // LF32: the card leads with its description (topic), not the raw id.
    await waitFor(() => expect(screen.getByText("Scheduling bulk-write")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Scheduling bulk-write"));
    // the drawer head also leads with the topic (it appears again inside the drawer)
    await waitFor(() =>
      expect(screen.getAllByText("Scheduling bulk-write").length).toBeGreaterThan(1),
    );
  });

  it("opens a needs_human card and fires retry through the recovery handler", async () => {
    render(<App />);
    await waitFor(() => expect(screen.getByText("Escalated item")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Escalated item"));
    // the failure action set renders (the audit trail's latest is an error).
    const retry = await screen.findByRole("button", { name: /retry/i });
    fireEvent.click(retry);
    await waitFor(() => expect(retryTaskMock).toHaveBeenCalledWith("T-99"));
  });

  it("surfaces a recovery-action failure as a dismissible alert (no unhandled rejection)", async () => {
    retryTaskMock.mockRejectedValueOnce(new Error("downstream is full"));
    render(<App />);
    await waitFor(() => expect(screen.getByText("Escalated item")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Escalated item"));
    const retry = await screen.findByRole("button", { name: /retry/i });
    fireEvent.click(retry);
    await waitFor(() => expect(screen.getByRole("alert").textContent).toMatch(/downstream is full/i));
    // dismissible
    fireEvent.click(screen.getByLabelText("dismiss action error"));
    await waitFor(() => expect(screen.queryByText(/downstream is full/i)).not.toBeInTheDocument());
  });

  it("docks the god terminal at the bottom", async () => {
    render(<App />);
    await waitFor(() => expect(screen.getByText(/claude/i)).toBeInTheDocument());
    expect(screen.getByPlaceholderText(/ask, inject, approve/i)).toBeInTheDocument();
  });
});
