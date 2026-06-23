import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { WiringStep } from "./WiringStep";
import { addTeam, emptyDraft } from "./draft";

describe("WiringStep", () => {
  it("renders teams + fork/join sections via PipelineView", () => {
    let d = addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B");
    d = { ...d, forks: [{ id: "fork-1", lanes: ["a", "b"] }], joins: [{ id: "join-1", waits_for: ["a", "b"], downstream: "needs-human" }] };
    render(<WiringStep draft={d} onChange={() => {}} />);
    expect(screen.getByText(/Forks \(1\)/)).toBeInTheDocument();
    expect(screen.getByText(/Joins \(1\)/)).toBeInTheDocument();
    expect(screen.getByText(/fork-1/)).toBeInTheDocument();
  });

  it("renders gates via PipelineView", () => {
    let d = addTeam(addTeam(emptyDraft(), "plan-writers", "Plan Writers"), "implementers", "Implementers");
    d = { ...d, gates: [{ id: "gate-2", label: "Plan review", downstream: "implementers" }] };
    render(<WiringStep draft={d} onChange={() => {}} />);
    expect(screen.getByText(/Gates \(1\)/)).toBeInTheDocument();
    expect(screen.getByText(/Plan review/)).toBeInTheDocument();
  });

  it("the add-gate control adds a gate and repoints the upstream team's on_approve", () => {
    const d = addTeam(addTeam(emptyDraft(), "plan-writers", "Plan Writers"), "implementers", "Implementers");
    const onChange = vi.fn();
    render(<WiringStep draft={d} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText(/gate id/i), { target: { value: "gate-2" } });
    fireEvent.change(screen.getByLabelText(/gate label/i), { target: { value: "Plan review" } });
    fireEvent.change(screen.getByLabelText(/gate upstream/i), { target: { value: "plan-writers" } });
    fireEvent.change(screen.getByLabelText(/gate downstream/i), { target: { value: "implementers" } });
    fireEvent.click(screen.getByRole("button", { name: /add gate/i }));
    const next = onChange.mock.calls.at(-1)?.[0];
    expect(next.gates).toEqual([{ id: "gate-2", label: "Plan review", downstream: "implementers" }]);
    expect(next.teams.find((t: { id: string }) => t.id === "plan-writers").outputs.on_approve).toBe("gate-2");
  });
});
