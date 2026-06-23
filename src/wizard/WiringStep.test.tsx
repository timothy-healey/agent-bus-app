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
    // fork-1 now appears in both the graph node and the form card.
    expect(screen.getAllByText(/fork-1/).length).toBeGreaterThan(0);
  });

  it("renders gates via PipelineView", () => {
    let d = addTeam(addTeam(emptyDraft(), "plan-writers", "Plan Writers"), "implementers", "Implementers");
    d = { ...d, gates: [{ id: "gate-2", label: "Plan review", downstream: "implementers" }] };
    render(<WiringStep draft={d} onChange={() => {}} />);
    expect(screen.getByText(/Gates \(1\)/)).toBeInTheDocument();
    // "Plan review" gate label appears in both the graph node and the form card.
    expect(screen.getAllByText(/Plan review/).length).toBeGreaterThan(0);
  });

  it("renders the add-fork control", () => {
    const d = addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B");
    render(<WiringStep draft={d} onChange={() => {}} />);
    expect(screen.getByLabelText(/fork lane 1/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/fork lane 2/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/fork downstream/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /add fork/i })).toBeInTheDocument();
  });

  it("the add-fork control adds a paired fork and join with 2 lanes + downstream", () => {
    const d = addTeam(addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B"), "c", "C");
    const onChange = vi.fn();
    render(<WiringStep draft={d} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText(/fork id/i), { target: { value: "1" } });
    fireEvent.change(screen.getByLabelText(/fork lane 1/i), { target: { value: "a" } });
    fireEvent.change(screen.getByLabelText(/fork lane 2/i), { target: { value: "b" } });
    fireEvent.change(screen.getByLabelText(/fork downstream/i), { target: { value: "c" } });
    fireEvent.click(screen.getByRole("button", { name: /add fork/i }));
    const next = onChange.mock.calls.at(-1)?.[0];
    expect(next.forks).toEqual([{ id: "fork-1", lanes: ["a", "b"] }]);
    expect(next.joins).toEqual([{ id: "join-1", waits_for: ["a", "b"], downstream: "c" }]);
  });

  it("the add-fork button is disabled until 2 distinct lanes + downstream + id are set", () => {
    const d = addTeam(addTeam(addTeam(emptyDraft(), "a", "A"), "b", "B"), "c", "C");
    render(<WiringStep draft={d} onChange={() => {}} />);
    const btn = screen.getByRole("button", { name: /add fork/i });
    expect(btn).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/fork id/i), { target: { value: "1" } });
    fireEvent.change(screen.getByLabelText(/fork lane 1/i), { target: { value: "a" } });
    fireEvent.change(screen.getByLabelText(/fork downstream/i), { target: { value: "c" } });
    // still only one lane chosen
    expect(btn).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/fork lane 2/i), { target: { value: "a" } });
    // two lanes but identical -> still blocked
    expect(btn).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/fork lane 2/i), { target: { value: "b" } });
    expect(btn).toBeEnabled();
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
