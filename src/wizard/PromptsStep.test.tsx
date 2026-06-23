import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { PromptsStep } from "./PromptsStep";
import { addTeam, emptyDraft, setPromptBody } from "./draft";

describe("PromptsStep", () => {
  it("renders an editor per team showing its body", () => {
    let d = addTeam(emptyDraft(), "research", "Research");
    d = setPromptBody(d, "research", "investigate the repo");
    render(<PromptsStep draft={d} onChange={() => {}} />);
    expect(screen.getByDisplayValue("investigate the repo")).toBeInTheDocument();
  });

  it("editing a body calls onChange with the new text", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<PromptsStep draft={d} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText(/prompt for research/i), { target: { value: "new responsibility" } });
    expect(onChange.mock.calls[0][0].teams[0].prompt_body).toBe("new responsibility");
  });
});
