import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { WiringStep } from "./WiringStep";
import { addTeam, emptyDraft } from "./draft";

describe("WiringStep", () => {
  it("renders teams + fork/join sections via PipelineView", () => {
    let d = addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B");
    d = { ...d, forks: [{ id: "fork-1", lanes: ["a", "b"] }], joins: [{ id: "join-1", waits_for: ["a", "b"], downstream: "needs-human" }] };
    render(<WiringStep draft={d} />);
    expect(screen.getByText(/Forks \(1\)/)).toBeInTheDocument();
    expect(screen.getByText(/Joins \(1\)/)).toBeInTheDocument();
    expect(screen.getByText(/fork-1/)).toBeInTheDocument();
  });

  it("renders gates via PipelineView", () => {
    let d = addTeam(addTeam(emptyDraft(), "plan-writers", "Plan Writers"), "implementers", "Implementers");
    d = { ...d, gates: [{ id: "gate-2", label: "Plan review", downstream: "implementers" }] };
    render(<WiringStep draft={d} />);
    expect(screen.getByText(/Gates \(1\)/)).toBeInTheDocument();
    expect(screen.getByText(/Plan review/)).toBeInTheDocument();
  });
});
