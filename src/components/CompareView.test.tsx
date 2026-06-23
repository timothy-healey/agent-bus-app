import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { CompareView } from "./CompareView";

describe("CompareView", () => {
  it("renders both pane labels and bodies side by side", () => {
    render(
      <CompareView
        left={{ label: "T-1-v1.md", markdown: "# Spec\n\noriginal body" }}
        right={{ label: "T-1-v2.md", markdown: "# Spec\n\nrevised body" }}
      />,
    );
    expect(screen.getByText("T-1-v1.md")).toBeInTheDocument();
    expect(screen.getByText("T-1-v2.md")).toBeInTheDocument();
    expect(screen.getByText(/original body/)).toBeInTheDocument();
    expect(screen.getByText(/revised body/)).toBeInTheDocument();
    // Two distinct rendered headings, one per pane.
    expect(screen.getAllByRole("heading", { level: 1, name: "Spec" })).toHaveLength(2);
  });

  it("shows a per-side change badge derived from the line diff", () => {
    render(
      <CompareView
        left={{ label: "v1", markdown: "a\nb\nc" }}
        right={{ label: "v2", markdown: "a\nx\nc\nd" }}
      />,
    );
    // right pane: +2 added (x, d), left pane: -1 removed (b)
    expect(screen.getByTestId("compare-badge-right")).toHaveTextContent("+2");
    expect(screen.getByTestId("compare-badge-left")).toHaveTextContent("-1");
  });

  it("renders empty-state text for an empty pane body", () => {
    render(
      <CompareView
        left={{ label: "v1", markdown: "" }}
        right={{ label: "v2", markdown: "# X" }}
      />,
    );
    expect(screen.getByText(/no artifact/i)).toBeInTheDocument();
  });
});
