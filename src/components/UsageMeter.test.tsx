import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { UsageMeter } from "./UsageMeter";
import type { UsageSnapshot } from "../ipc/usage";

const base: UsageSnapshot = {
  window_total: 66_500_000, window_budget: 190_000_000, window_pct: 0.35,
  band: "safe", burn_per_min: 18_000, window_secs: 18000,
  reset_in_secs: null, est_brake_at: null,
  by_team: [{ team_id: "research", tokens: 540_000 }], tokens_by_task: {}, braked: false, auto_meter_enabled: false,
};

describe("UsageMeter", () => {
  it("renders nothing-ready placeholder when snapshot is null", () => {
    const { container } = render(<UsageMeter snapshot={null} />);
    expect(container.querySelector("[data-testid='usage-meter']")).not.toBeNull();
  });

  it("shows pct, window line and burn for a safe snapshot", () => {
    render(<UsageMeter snapshot={base} />);
    expect(screen.getByText("35%")).toBeInTheDocument();
    expect(screen.getByText(/5h window · 66.5M tok/)).toBeInTheDocument();
    expect(screen.getByText(/18k\/min/)).toBeInTheDocument();
  });

  it("shows the reset countdown (not burn) when braked", () => {
    render(<UsageMeter snapshot={{ ...base, braked: true, band: "braked", reset_in_secs: 1440 }} />);
    expect(screen.getByText(/↻ 24m/)).toBeInTheDocument();
    expect(screen.queryByText(/\/min/)).toBeNull();
  });

  it("renders the per-team breakdown in the tooltip", () => {
    render(<UsageMeter snapshot={base} />);
    expect(screen.getByText("research")).toBeInTheDocument();
  });

  it("paints the spec'd gradient fill, not a solid band (Decision 6)", () => {
    render(<UsageMeter snapshot={base} />);
    const fill = screen.getByTestId("usage-bar-fill");
    expect(fill.style.background).toContain("linear-gradient");
    expect(fill.style.background).toContain("--running");
    expect(fill.style.background).toContain("--warn");
    expect(fill.style.background).toContain("--danger");
  });

  it("is keyboard-reachable and exposes a progressbar role (A5)", () => {
    render(<UsageMeter snapshot={base} />);
    expect(screen.getByTestId("usage-meter").getAttribute("tabindex")).toBe("0");
    const bar = screen.getByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBe("35");
  });

  it("shows the brake-ETA row when est_brake_at is in the future", () => {
    // now passed explicitly for determinism; est_brake_at 30 min ahead
    render(<UsageMeter snapshot={{ ...base, est_brake_at: 1000 + 1800 }} now={1000} />);
    expect(screen.getByText(/brake at budget/i)).toBeInTheDocument();
    expect(screen.getByText(/~30min/)).toBeInTheDocument();
  });

  it("omits the brake-ETA row when est_brake_at is null", () => {
    render(<UsageMeter snapshot={{ ...base, est_brake_at: null }} now={1000} />);
    expect(screen.queryByText(/brake at budget/i)).toBeNull();
  });

  it("shows a window-reset row when reset_in_secs is present and not braked", () => {
    render(<UsageMeter snapshot={{ ...base, reset_in_secs: 1440 }} now={1000} />);
    expect(screen.getByText(/window resets in/i)).toBeInTheDocument();
    expect(screen.getByText(/↻ 24m/)).toBeInTheDocument();
  });

  it("renders every team row in the tooltip (complete breakdown)", () => {
    render(
      <UsageMeter
        snapshot={{ ...base, by_team: [
          { team_id: "research", tokens: 540_000 },
          { team_id: "writers", tokens: 120_000 },
          { team_id: "reviewers", tokens: 30_000 },
        ] }}
        now={1000}
      />,
    );
    expect(screen.getByText("research")).toBeInTheDocument();
    expect(screen.getByText("writers")).toBeInTheDocument();
    expect(screen.getByText("reviewers")).toBeInTheDocument();
  });
});
