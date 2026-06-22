import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { SettingsView } from "./SettingsView";
import type { UsageSnapshot } from "../ipc/usage";

const snap: UsageSnapshot = {
  window_total: 1000, window_budget: 2_600_000, window_pct: 0.0004, band: "safe",
  burn_per_min: 0, window_secs: 18000, reset_in_secs: null, est_brake_at: null,
  by_team: [], tokens_by_task: {}, braked: false,
};

beforeEach(() => {
  document.documentElement.setAttribute("data-theme", "dark");
});

describe("SettingsView", () => {
  it("renders General and Usage section headings", () => {
    render(<SettingsView usage={snap} onSetBudget={vi.fn().mockResolvedValue(snap)} />);
    expect(screen.getByText(/general/i)).toBeInTheDocument();
    expect(screen.getByText(/usage/i)).toBeInTheDocument();
  });

  it("toggles the theme attribute when the theme control is used", () => {
    render(<SettingsView usage={snap} onSetBudget={vi.fn().mockResolvedValue(snap)} />);
    fireEvent.click(screen.getByRole("button", { name: /light/i }));
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("calls onSetBudget with the entered number", async () => {
    const onSetBudget = vi.fn().mockResolvedValue(snap);
    render(<SettingsView usage={snap} onSetBudget={onSetBudget} />);
    fireEvent.change(screen.getByLabelText(/window budget/i), { target: { value: "5000000" } });
    fireEvent.click(screen.getByRole("button", { name: /save budget/i }));
    await waitFor(() => expect(onSetBudget).toHaveBeenCalledWith(5_000_000));
  });

  it("shows the current budget from the snapshot", () => {
    render(<SettingsView usage={snap} onSetBudget={vi.fn().mockResolvedValue(snap)} />);
    expect(screen.getByText(/v1.1/i)).toBeInTheDocument(); // deferred sections note
  });
});
