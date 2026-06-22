import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { UsageMeter } from "./UsageMeter";
import type { UsageSnapshot } from "../ipc/usage";

const base: UsageSnapshot = {
  window_total: 1_200_000, window_budget: 2_600_000, window_pct: 0.47,
  band: "safe", burn_per_min: 18_000, window_secs: 18000,
  reset_in_secs: null, est_brake_at: null,
  by_team: [{ team_id: "research", tokens: 540_000 }], tokens_by_task: {}, braked: false,
};

describe("UsageMeter", () => {
  it("renders nothing-ready placeholder when snapshot is null", () => {
    const { container } = render(<UsageMeter snapshot={null} />);
    expect(container.querySelector("[data-testid='usage-meter']")).not.toBeNull();
  });

  it("shows pct, window line and burn for a safe snapshot", () => {
    render(<UsageMeter snapshot={base} />);
    expect(screen.getByText("47%")).toBeInTheDocument();
    expect(screen.getByText(/5h window · 1.2M tok/)).toBeInTheDocument();
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
});
