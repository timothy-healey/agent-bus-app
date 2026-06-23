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
});
