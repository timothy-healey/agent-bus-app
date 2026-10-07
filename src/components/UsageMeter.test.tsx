import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { UsageMeter } from "./UsageMeter";
import type { UsageSnapshot } from "../ipc/usage";

const snap = (over: Partial<UsageSnapshot> = {}): UsageSnapshot => ({
  available: true,
  observed_at: 1_000,
  session: { label: "session (5h)", utilization_pct: 41, resets_in_secs: 3_600 },
  weekly: { label: "weekly (7d)", utilization_pct: 2, resets_in_secs: 86_400 },
  model_scoped: [{ label: "Fable weekly", utilization_pct: 0, resets_in_secs: 86_400 }],
  band: "safe",
  braked: false,
  auto_meter_enabled: false,
  by_team: [{ team_id: "research", tokens: 240, cost_usd: 0.25 }],
  tokens_by_task: {},
  ...over,
});

describe("UsageMeter", () => {
  it("headlines the session Utilization", () => {
    render(<UsageMeter snapshot={snap()} now={1_010} />);
    expect(screen.getByRole("group", { name: "usage 41% of session limit" })).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "41");
  });

  it("shows a dash when there has never been a reading", () => {
    render(<UsageMeter snapshot={snap({ observed_at: null, session: null, weekly: null, model_scoped: [], available: false })} now={1_010} />);
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("says usage unavailable and shows the reading's age when the last poll failed", () => {
    render(<UsageMeter snapshot={snap({ available: false })} now={1_010} />);
    expect(screen.getByText(/— usage unavailable/)).toBeInTheDocument();
    expect(screen.getAllByText(/as of/).length).toBeGreaterThan(0);
  });

  it("shows 'as of' once a good reading is over 3 minutes old", () => {
    render(<UsageMeter snapshot={snap()} now={1_000 + 600} />);
    expect(screen.getAllByText(/as of/).length).toBeGreaterThan(0);
  });

  it("tooltip lists both windows, model-scoped limits and per-team cost", () => {
    render(<UsageMeter snapshot={snap()} now={1_010} />);
    expect(screen.getByText("session (5h)")).toBeInTheDocument();
    expect(screen.getByText("weekly (7d)")).toBeInTheDocument();
    expect(screen.getByText("Fable weekly")).toBeInTheDocument();
    expect(screen.getByText("cost this 5h (list price)")).toBeInTheDocument();
    expect(screen.getByText(/\$0\.25/)).toBeInTheDocument();
  });
});
