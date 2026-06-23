import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { TeamsStep } from "./TeamsStep";
import { addTeam, emptyDraft } from "./draft";

describe("TeamsStep", () => {
  it("renders a card per team", () => {
    const d = addTeam(addTeam(emptyDraft(), "research", "Research"), "writers", "Writers");
    render(<TeamsStep draft={d} onChange={() => {}} />);
    expect(screen.getByDisplayValue("Research")).toBeInTheDocument();
    expect(screen.getByDisplayValue("Writers")).toBeInTheDocument();
  });

  it("renaming a team calls onChange with the new name", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.change(screen.getByDisplayValue("Research"), { target: { value: "Investigators" } });
    expect(onChange).toHaveBeenCalled();
    const next = onChange.mock.calls[0][0];
    expect(next.teams[0].name).toBe("Investigators");
  });

  it("removing a team calls onChange without it", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /remove research/i }));
    expect(onChange.mock.calls[0][0].teams).toHaveLength(0);
  });

  it("the advanced panel edits the model", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /advanced research/i }));
    fireEvent.change(screen.getByLabelText(/model for research/i), { target: { value: "claude-haiku-4" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].runner.model).toBe("claude-haiku-4");
  });

  it("the advanced panel sets an effort preset", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /advanced research/i }));
    fireEvent.change(screen.getByLabelText(/effort for research/i), { target: { value: "extended-high" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].runner.effort).toEqual({ mode: "extended-high" });
  });

  it("choosing custom effort reveals a budget input that sets budget_tokens", () => {
    const d = { ...addTeam(emptyDraft(), "research", "Research") };
    d.teams[0].runner.effort = { mode: "custom", budget_tokens: 12000 };
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /advanced research/i }));
    fireEvent.change(screen.getByLabelText(/budget for research/i), { target: { value: "20000" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].runner.effort).toEqual({ mode: "custom", budget_tokens: 20000 });
  });

  it("the advanced panel edits tools / reads / writes", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /advanced research/i }));
    fireEvent.change(screen.getByLabelText(/tools for research/i), { target: { value: "Read, Grep" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].scope.tools).toEqual(["Read", "Grep"]);
    fireEvent.change(screen.getByLabelText(/reads for research/i), { target: { value: "src/**" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].scope.reads).toEqual(["src/**"]);
    fireEvent.change(screen.getByLabelText(/writes for research/i), { target: { value: "artifacts/**" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].scope.writes).toEqual(["artifacts/**"]);
  });
});
