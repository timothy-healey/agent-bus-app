import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
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
      workers: { default: 1, max: 3 },
    },
  ],
  gates: [{ id: "gate-1-spec", label: "Gate 1 — Spec Approval", downstream: "planners" }],
  escalations: [{ id: "needs-human", triggers: ["attempts >= 3"] }],
};

describe("PipelineView", () => {
  it("renders the pipeline name + description", () => {
    render(<PipelineView pipeline={pipeline} />);
    expect(screen.getByText("DDD Spec → Plan → Implement")).toBeInTheDocument();
    expect(screen.getByText("Two human gates.")).toBeInTheDocument();
  });

  it("renders each team with model + effort", () => {
    render(<PipelineView pipeline={pipeline} />);
    expect(screen.getByText("Research")).toBeInTheDocument();
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
    expect(screen.getByText("needs-human")).toBeInTheDocument();
    expect(screen.getByText(/attempts >= 3/)).toBeInTheDocument();
  });

  it("shows an empty-state when pipeline is null", () => {
    render(<PipelineView pipeline={null} />);
    expect(screen.getByText(/no pipeline/i)).toBeInTheDocument();
  });
});
