import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { PipelineView } from "./PipelineView";
import type { Pipeline } from "../ipc/pipeline";

const pipeline: Pipeline = {
  id: "ddd-spec-plan-impl",
  name: "DDD Spec → Plan → Implement",
  description: "Two human gates.",
  schema_version: 1,
  teams: [
    {
      id: "research",
      name: "Research",
      prompt: "prompts/research.md",
      runner: { kind: "claude-cli", model: "claude-opus-4-7", effort: { mode: "extended-high" } },
      scope: { reads: ["${target_repo}"], writes: ["${project}/artifacts/analyses"], tools: ["Read"] },
      outputs: { on_approve: "spec-writers", on_revise: null, on_reject: null },
      workers: { min: 1, max: 3 },
      role: "producer",
      store: { capacity: 8 },
    },
  ],
  gates: [{ id: "gate-1-spec", label: "Gate 1 — Spec Approval", downstream: "planners" }],
  escalations: [{ id: "needs-human", triggers: ["attempts >= 3"] }],
  forks: [],
  joins: [],
};

describe("PipelineView", () => {
  it("renders the pipeline name + description", () => {
    render(<PipelineView pipeline={pipeline} />);
    expect(screen.getByText("DDD Spec → Plan → Implement")).toBeInTheDocument();
    expect(screen.getByText("Two human gates.")).toBeInTheDocument();
  });

  it("renders each team with model + effort", () => {
    render(<PipelineView pipeline={pipeline} />);
    // "Research" appears in both the graph node and the form card.
    expect(screen.getAllByText("Research").length).toBeGreaterThan(0);
    expect(screen.getByText(/claude-opus-4-7/)).toBeInTheDocument();
    expect(screen.getByText(/extended-high/)).toBeInTheDocument();
  });

  it("renders a team's approve route target", () => {
    render(<PipelineView pipeline={pipeline} />);
    expect(screen.getByText(/spec-writers/)).toBeInTheDocument();
  });

  it("renders gates with their downstream", () => {
    render(<PipelineView pipeline={pipeline} />);
    expect(screen.getByText("Gate 1 — Spec Approval")).toBeInTheDocument();
  });

  it("renders escalations with triggers", () => {
    render(<PipelineView pipeline={pipeline} />);
    // "needs-human" appears in both the graph node and the escalation card.
    expect(screen.getAllByText("needs-human").length).toBeGreaterThan(0);
    expect(screen.getByText(/attempts >= 3/)).toBeInTheDocument();
  });

  it("renders a read-only static graph honouring node roles + edges (Decision 2)", () => {
    const p: Pipeline = {
      id: "p", name: "Flow", description: "", schema_version: 1,
      teams: [
        { id: "writer", name: "Writer", prompt: "", scope: { reads: [], writes: [], tools: [] }, outputs: { on_approve: "gate-1" }, workers: { min: 1, max: 1 }, role: "producer", store: { capacity: 8 } },
      ],
      gates: [{ id: "gate-1", label: "Gate 1", downstream: "writer" }],
      escalations: [], forks: [], joins: [],
    };
    const { container } = render(<PipelineView pipeline={p} />);
    expect(screen.getByTestId("pipeline-graph")).toBeInTheDocument();
    expect(container.querySelector('[data-node-role="gate"]')).not.toBeNull();
    expect(container.querySelector('[data-edge-kind="hand-off"]')).not.toBeNull();
  });

  it("shows an empty-state when pipeline is null", () => {
    render(<PipelineView pipeline={null} />);
    expect(screen.getByText(/no pipeline/i)).toBeInTheDocument();
  });

  it("renders fork lanes and join downstream", () => {
    const p: Pipeline = {
      id: "p",
      name: "Parallel",
      description: "",
      schema_version: 2,
      teams: [],
      gates: [],
      escalations: [],
      forks: [{ id: "fork-1", lanes: ["lane-a", "lane-b"] }],
      joins: [{ id: "join-1", waits_for: ["lane-a", "lane-b"], downstream: "after" }],
    };
    render(<PipelineView pipeline={p} />);
    // fork-1 / join-1 now appear in both the graph and the form cards.
    expect(screen.getAllByText(/fork-1/).length).toBeGreaterThan(0);
    expect(screen.getByText(/lanes: lane-a, lane-b/)).toBeInTheDocument();
    expect(screen.getAllByText(/join-1/).length).toBeGreaterThan(0);
    expect(screen.getByText(/→ after/)).toBeInTheDocument();
  });

  it("renders an Edit pipeline button when onEdit is provided and calls it", () => {
    const onEdit = vi.fn();
    render(<PipelineView pipeline={pipeline} onEdit={onEdit} />);
    fireEvent.click(screen.getByRole("button", { name: /edit pipeline/i }));
    expect(onEdit).toHaveBeenCalledTimes(1);
  });

  it("shows no edit button without onEdit (pure viewer)", () => {
    render(<PipelineView pipeline={pipeline} />);
    expect(screen.queryByRole("button", { name: /edit pipeline/i })).toBeNull();
  });
});
