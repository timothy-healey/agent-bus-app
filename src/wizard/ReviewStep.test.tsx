import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ReviewStep } from "./ReviewStep";
import { addTeam, emptyDraft, setPromptBody } from "./draft";

describe("ReviewStep", () => {
  function draft() {
    let d = addTeam(emptyDraft(), "research", "Research");
    d = setPromptBody(d, "research", "investigate");
    return { ...d, name: "Demo" };
  }

  it("renders the assembled pipeline + the prompt files", () => {
    render(<ReviewStep basics={{ name: "Demo", root: "/p", description: "", targetRepo: "" }} draft={draft()} />);
    expect(screen.getByText("investigate")).toBeInTheDocument();
    expect(screen.getByText(/prompts\/research\.md/)).toBeInTheDocument();
  });

  it("renders the backend error alert when given an error", () => {
    render(
      <ReviewStep
        basics={{ name: "Demo", root: "/p", description: "", targetRepo: "" }}
        draft={draft()}
        error="team 'research' is unreachable"
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent(/unreachable/);
  });

  it("is presentational — renders no Create button", () => {
    render(<ReviewStep basics={{ name: "Demo", root: "/p", description: "", targetRepo: "" }} draft={draft()} />);
    expect(screen.queryByRole("button", { name: /create/i })).not.toBeInTheDocument();
  });
  it("shows the app-managed location and the target repo", () => {
    render(<ReviewStep basics={{ name: "Demo", root: "", description: "", targetRepo: "/repo" }} draft={draft()} />);
    expect(screen.getByText(/location: app-managed/i)).toBeInTheDocument();
    expect(screen.getByText(/target repo: \/repo/i)).toBeInTheDocument();
  });
});
