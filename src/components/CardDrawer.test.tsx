import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";

const listMock = vi.fn();
const addMock = vi.fn();
const delMock = vi.fn();
const reanchorMock = vi.fn();
vi.mock("../ipc/review", () => ({
  listComments: (...a: unknown[]) => listMock(...a),
  addComment: (...a: unknown[]) => addMock(...a),
  deleteComment: (...a: unknown[]) => delMock(...a),
  reanchorComments: (...a: unknown[]) => reanchorMock(...a),
}));

import { CardDrawer } from "./CardDrawer";
import type { InvocationRow, Task } from "../ipc/runtime";
import type { Pipeline } from "../ipc/pipeline";

function task(over: Partial<Task> = {}): Task {
  return {
    id: "T-40", project_id: "p", pipeline: "p", topic: "Scheduling bulk-write",
    target_repo: null, target_scope: null, current_stage: "gate-2-plan",
    state: "gated", attempts: 1, parent_artifact: null, review_artifact: null,
    created_at: 0, updated_at: 0, ...over,
  };
}

function inv(over: Partial<InvocationRow> = {}): InvocationRow {
  return {
    invocation_id: "I-1", team_id: "spec", model: "claude-opus-4-8", attempts: 1,
    started_at: 100, settled_at: 200, outcome: "error:rate_limited",
    input_tokens: 80, output_tokens: 20, ...over,
  };
}

const escalationPipeline: Pipeline = {
  id: "p", name: "P", description: "", schema_version: 3,
  teams: [], gates: [], escalations: [{ id: "needs-human", triggers: [] }], forks: [], joins: [],
};

describe("CardDrawer", () => {
  beforeEach(() => {
    listMock.mockReset().mockResolvedValue([]);
    addMock.mockReset();
    delMock.mockReset();
    reanchorMock.mockReset().mockResolvedValue([]);
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

  it("shows a loading skeleton in the live log while running with no output (S1)", () => {
    render(
      <CardDrawer task={task({ state: "running" })} artifactMarkdown="" logText="" onApprove={() => {}} onRevise={() => {}} onReject={() => {}} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /live log/i }));
    expect(screen.getByTestId("live-log").getAttribute("data-log-state")).toBe("loading");
  });

  it("shows a streaming caret in the live log while running with output (S1)", () => {
    render(
      <CardDrawer task={task({ state: "running" })} artifactMarkdown="" logText="working on it" onApprove={() => {}} onRevise={() => {}} onReject={() => {}} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /live log/i }));
    const log = screen.getByTestId("live-log");
    expect(log.getAttribute("data-log-state")).toBe("streaming");
    expect(log.textContent).toContain("working on it");
  });

  it("renders output as prose and thinking dimmed with a marker", () => {
    render(
      <CardDrawer
        task={task({ state: "running" })}
        artifactMarkdown=""
        logSegments={[
          { kind: "thinking", text: "let me reason" },
          { kind: "output", text: "the answer" },
        ]}
        onApprove={() => {}}
        onRevise={() => {}}
        onReject={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /live log/i }));
    const out = screen.getByText("the answer");
    expect(out).toBeInTheDocument();
    const thinking = screen.getByText(/let me reason/);
    // thinking is tagged for distinct styling + carries the marker
    expect(thinking.closest("[data-log-kind='thinking']")).not.toBeNull();
    expect(screen.getByText(/thinking/i)).toBeInTheDocument();
  });

  it("renders an error treatment when the log contains an error (S1)", () => {
    render(
      <CardDrawer task={task({ state: "gated" })} artifactMarkdown="" logText="[error] runner crashed" onApprove={() => {}} onRevise={() => {}} onReject={() => {}} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /live log/i }));
    const log = screen.getByTestId("live-log");
    expect(log.getAttribute("data-log-state")).toBe("error");
    expect(screen.getByRole("alert").textContent).toContain("runner crashed");
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

  it("raises onCompare when two versions are picked in the lineage tab", () => {
    const onCompare = vi.fn();
    render(
      <CardDrawer
        task={task({ parent_artifact: "artifacts/T-40-v1.md", review_artifact: "artifacts/T-40-rev.md" })}
        artifactMarkdown="# Plan"
        onCompare={onCompare}
        onApprove={() => {}}
        onRevise={() => {}}
        onReject={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /lineage/i }));
    fireEvent.click(screen.getByRole("button", { name: /^compare/i }));
    fireEvent.click(screen.getByRole("checkbox", { name: /T-40-v1\.md/ }));
    fireEvent.click(screen.getByRole("checkbox", { name: /T-40-rev\.md/ }));
    expect(onCompare).toHaveBeenCalledWith("artifacts/T-40-v1.md", "artifacts/T-40-rev.md");
  });

  // ---- plan H: history panel + state-aware actions ----

  function base() {
    return { onApprove: vi.fn(), onRevise: vi.fn(), onReject: vi.fn(),
      onRetry: vi.fn(), onForceAdvance: vi.fn(), onAbandon: vi.fn(), onAccept: vi.fn() };
  }

  it("gated card shows reject/revise/approve (unchanged) and no recovery actions", () => {
    render(<CardDrawer task={task({ state: "gated" })} artifactMarkdown="# Plan" {...base()} />);
    expect(screen.getByTestId("action-bar").getAttribute("data-mode")).toBe("gated");
    expect(screen.getByRole("button", { name: /^reject/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^revise/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^approve$/i })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /retry/i })).not.toBeInTheDocument();
  });

  it("needs_human failure shows retry / approve and advance / abandon", () => {
    render(
      <CardDrawer
        task={task({ state: "needs_human", current_stage: "needs-human" })}
        artifactMarkdown="# Plan"
        invocations={[inv({ outcome: "error:model_unavailable" })]}
        pipeline={escalationPipeline}
        {...base()}
      />,
    );
    expect(screen.getByTestId("action-bar").getAttribute("data-mode")).toBe("needs_human:failure");
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /approve and advance/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^abandon/i })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /accept/i })).not.toBeInTheDocument();
  });

  it("abandon requires a confirm before firing onAbandon", () => {
    const b = base();
    render(
      <CardDrawer task={task({ state: "needs_human" })} artifactMarkdown="# Plan"
        invocations={[inv({ outcome: "verdict:reject" })]} pipeline={escalationPipeline} {...b} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /^abandon/i }));
    expect(b.onAbandon).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /confirm abandon/i }));
    expect(b.onAbandon).toHaveBeenCalledWith("T-40");
  });

  it("retry + approve-and-advance fire their handlers", () => {
    const b = base();
    render(
      <CardDrawer task={task({ state: "needs_human" })} artifactMarkdown="# Plan"
        invocations={[inv({ outcome: "error:other" })]} pipeline={escalationPipeline} {...b} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(b.onRetry).toHaveBeenCalledWith("T-40");
    fireEvent.click(screen.getByRole("button", { name: /approve and advance/i }));
    expect(b.onForceAdvance).toHaveBeenCalledWith("T-40");
  });

  it("needs_human hand-off shows accept + send back (not retry/abandon)", () => {
    render(
      <CardDrawer task={task({ state: "needs_human" })} artifactMarkdown="# Plan"
        invocations={[inv({ outcome: "verdict:approve", team_id: "writers" })]}
        pipeline={escalationPipeline} {...base()} />,
    );
    expect(screen.getByTestId("action-bar").getAttribute("data-mode")).toBe("needs_human:handoff");
    expect(screen.getByRole("button", { name: /accept/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /send back/i })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /retry/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^abandon/i })).not.toBeInTheDocument();
  });

  it("accept fires onAccept", () => {
    const b = base();
    render(
      <CardDrawer task={task({ state: "needs_human" })} artifactMarkdown="# Plan"
        invocations={[inv({ outcome: "verdict:approve" })]} pipeline={escalationPipeline} {...b} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /accept/i }));
    expect(b.onAccept).toHaveBeenCalledWith("T-40");
  });

  it("a non-gated non-needs_human card is read-only", () => {
    render(<CardDrawer task={task({ state: "running" })} artifactMarkdown="# Plan" {...base()} />);
    expect(screen.getByTestId("action-bar").getAttribute("data-mode")).toBe("read-only");
    expect(screen.getByText(/read only/i)).toBeInTheDocument();
  });

  it("surfaces the latest outcome as the headline reason line for needs_human", () => {
    render(
      <CardDrawer task={task({ state: "needs_human" })} artifactMarkdown="# Plan"
        invocations={[inv({ outcome: "error:model_unavailable", team_id: "spec" })]}
        pipeline={escalationPipeline} {...base()} />,
    );
    const reason = screen.getByTestId("reason-line");
    expect(reason.getAttribute("data-kind")).toBe("failure");
    expect(reason.textContent).toMatch(/model unavailable/i);
    expect(reason.textContent).toMatch(/spec/);
  });

  it("lists invocations in the history panel with usage + outcome", () => {
    render(
      <CardDrawer task={task({ state: "needs_human" })} artifactMarkdown="# Plan"
        invocations={[inv({ outcome: "verdict:reject", team_id: "reviewers", input_tokens: 80, output_tokens: 20 })]}
        pipeline={escalationPipeline} now={500} {...base()} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /history/i }));
    const panel = screen.getByTestId("history-panel");
    expect(within(panel).getByText("reviewers")).toBeInTheDocument();
    expect(within(panel).getByText(/rejected/i)).toBeInTheDocument();
    expect(within(panel).getByText(/tokens/i)).toBeInTheDocument();
  });

  it("shows an empty history hint when there are no invocations", () => {
    render(
      <CardDrawer task={task({ state: "needs_human" })} artifactMarkdown="# Plan"
        invocations={[]} pipeline={escalationPipeline} {...base()} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /history/i }));
    expect(screen.getByText(/no invocations recorded/i)).toBeInTheDocument();
  });

  it("renders the compare view in the lineage tab when compareMarkdown is supplied", () => {
    render(
      <CardDrawer
        task={task({ parent_artifact: "artifacts/T-40-v1.md", review_artifact: "artifacts/T-40-rev.md" })}
        artifactMarkdown="# Plan"
        compareMarkdown={{ left: "# Plan\n\nv1 body", right: "# Plan\n\nrev body" }}
        compareLabels={{ left: "T-40-v1.md", right: "T-40-rev.md" }}
        onApprove={() => {}}
        onRevise={() => {}}
        onReject={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /lineage/i }));
    expect(screen.getByTestId("compare-view")).toBeInTheDocument();
    expect(screen.getByText(/v1 body/)).toBeInTheDocument();
    expect(screen.getByText(/rev body/)).toBeInTheDocument();
  });
});
