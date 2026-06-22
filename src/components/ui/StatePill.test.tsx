import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { StatePill, stateLabel } from "./StatePill";

describe("stateLabel", () => {
  it("maps gated to the needs-you label", () => {
    expect(stateLabel("gated")).toBe("needs you");
  });
  it("maps needs_human to needs human", () => {
    expect(stateLabel("needs_human")).toBe("needs human");
  });
  it("maps revising to revise", () => {
    expect(stateLabel("revising")).toBe("revise");
  });
  it("passes other states through", () => {
    expect(stateLabel("running")).toBe("running");
    expect(stateLabel("queued")).toBe("queued");
  });
});

describe("StatePill", () => {
  it("renders the label for the state", () => {
    render(<StatePill state="running" />);
    expect(screen.getByText("running")).toBeInTheDocument();
  });

  it("renders a dot element", () => {
    const { container } = render(<StatePill state="gated" />);
    expect(container.querySelector("[data-dot]")).toBeTruthy();
  });

  it("applies the pulse class only to the running dot", () => {
    const { container, rerender } = render(<StatePill state="running" />);
    expect(container.querySelector(".abp-pulse")).not.toBeNull();
    rerender(<StatePill state="queued" />);
    expect(container.querySelector(".abp-pulse")).toBeNull();
  });
});
