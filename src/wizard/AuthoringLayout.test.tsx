import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { AuthoringLayout, type NavStep } from "./AuthoringLayout";

function steps(onSelect: (id: string) => void): NavStep[] {
  return [
    { id: "basics", label: "Basics", onSelect: () => onSelect("basics") },
    { id: "canvas", label: "Canvas", onSelect: () => onSelect("canvas") },
    { id: "review", label: "Review", onSelect: () => onSelect("review"), disabled: true },
  ];
}

describe("AuthoringLayout (G14 left-nav tree)", () => {
  it("renders the nav tree as a list and marks the current step", () => {
    render(
      <AuthoringLayout title="New project" steps={steps(() => {})} currentStepId="canvas" onClose={() => {}}>
        <div>pane</div>
      </AuthoringLayout>,
    );
    // current step carries aria-current=step; others do not.
    const current = screen.getByRole("button", { name: "Canvas" });
    expect(current).toHaveAttribute("aria-current", "step");
    expect(screen.getByRole("button", { name: "Basics" })).not.toHaveAttribute("aria-current");
  });

  it("clicking a step jumps directly to it (not just next/back)", () => {
    const onSelect = vi.fn();
    render(
      <AuthoringLayout title="t" steps={steps(onSelect)} currentStepId="basics" onClose={() => {}}>
        <div>pane</div>
      </AuthoringLayout>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Canvas" }));
    expect(onSelect).toHaveBeenCalledWith("canvas");
  });

  it("a disabled step is non-interactive (gating reserved for B2)", () => {
    const onSelect = vi.fn();
    render(
      <AuthoringLayout title="t" steps={steps(onSelect)} currentStepId="basics" onClose={() => {}}>
        <div>pane</div>
      </AuthoringLayout>,
    );
    const review = screen.getByRole("button", { name: "Review" });
    expect(review).toBeDisabled();
    fireEvent.click(review);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("renders the switcher and footer slots and the main pane", () => {
    render(
      <AuthoringLayout
        title="t"
        steps={steps(() => {})}
        currentStepId="basics"
        switcher={<div>SWITCHER</div>}
        footer={<button type="button">Continue</button>}
        onClose={() => {}}
      >
        <div>MAIN PANE</div>
      </AuthoringLayout>,
    );
    expect(screen.getByText("SWITCHER")).toBeInTheDocument();
    expect(screen.getByText("MAIN PANE")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeInTheDocument();
  });

  it("Escape closes the view (full-page exit)", () => {
    const onClose = vi.fn();
    render(
      <AuthoringLayout title="t" steps={steps(() => {})} currentStepId="basics" onClose={onClose}>
        <div>pane</div>
      </AuthoringLayout>,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("the close button calls onClose", () => {
    const onClose = vi.fn();
    render(
      <AuthoringLayout title="t" steps={steps(() => {})} currentStepId="basics" onClose={onClose}>
        <div>pane</div>
      </AuthoringLayout>,
    );
    fireEvent.click(screen.getByRole("button", { name: /close authoring view/i }));
    expect(onClose).toHaveBeenCalled();
  });
});
