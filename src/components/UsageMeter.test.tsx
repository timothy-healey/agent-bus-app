import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
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
  last_error: null,
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

  const headline = () => within(screen.getByTestId("usage-headline"));

  it("shows no 'as of' in the headline or tooltip for a fresh available reading", () => {
    render(<UsageMeter snapshot={snap()} now={1_010} />);
    expect(headline().queryByText(/as of/)).toBeNull();
    expect(screen.queryByText("reading")).toBeNull();
  });

  it("shows 'as of' in the headline once a good reading is over 3 minutes old", () => {
    render(<UsageMeter snapshot={snap()} now={1_000 + 600} />);
    expect(headline().getByText(/as of/)).toBeInTheDocument();
  });

  it("says usage unavailable and shows the reading's age when the last poll failed", () => {
    render(<UsageMeter snapshot={snap({ available: false })} now={1_010} />);
    expect(headline().getByText(/— usage unavailable/)).toBeInTheDocument();
    expect(headline().getByText(/as of/)).toBeInTheDocument();
  });

  it("shows why the last poll failed in the tooltip", () => {
    const { container } = render(
      <UsageMeter snapshot={snap({ available: false, last_error: "usage query timed out" })} now={1_010} />,
    );
    const tooltip = within(container.querySelector(".usage-tooltip") as HTMLElement);
    expect(tooltip.getByText("error")).toBeInTheDocument();
    expect(tooltip.getByText("usage query timed out")).toBeInTheDocument();
  });

  it("says usage unavailable even when the failed poll left no session reading", () => {
    render(<UsageMeter snapshot={snap({ available: false, session: null })} now={1_010} />);
    expect(headline().getByText(/— usage unavailable/)).toBeInTheDocument();
  });

  it("reports unknown to assistive tech when there is no session reading", () => {
    render(<UsageMeter snapshot={snap({ session: null })} now={1_010} />);
    expect(screen.getByRole("group", { name: "usage unavailable" })).toBeInTheDocument();
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuetext", "unknown");
    expect(bar).not.toHaveAttribute("aria-valuenow");
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
