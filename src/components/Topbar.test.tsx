import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Topbar } from "./Topbar";
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

describe("Topbar", () => {
  it("shows the brand mark", () => {
    render(<Topbar activeProject={null} onNewProject={() => {}} usage={null} brakeOn={false} onToggleBrake={() => {}} />);
    expect(screen.getByText(/agent bus/i)).toBeInTheDocument();
  });

  it("shows 'No project' pill when activeProject is null", () => {
    render(<Topbar activeProject={null} onNewProject={() => {}} usage={null} brakeOn={false} onToggleBrake={() => {}} />);
    expect(screen.getByText(/no project/i)).toBeInTheDocument();
  });

  it("renders the usage meter pct when a snapshot is given", () => {
    render(<Topbar activeProject={null} onNewProject={() => {}} usage={snap()} brakeOn={false} onToggleBrake={() => {}} />);
    expect(screen.getByText("41%")).toBeInTheDocument();
  });

  it("shows brake off and toggles on click", () => {
    const onToggle = vi.fn();
    render(<Topbar activeProject={null} onNewProject={() => {}} usage={snap()} brakeOn={false} onToggleBrake={onToggle} />);
    const brake = screen.getByRole("button", { name: /brake off/i });
    fireEvent.click(brake);
    expect(onToggle).toHaveBeenCalledWith(true);
  });

  it("shows brake on with reason hint when braked", () => {
    render(<Topbar activeProject={null} onNewProject={() => {}} usage={{ ...snap(), braked: true, band: "braked" }} brakeOn={true} brakeReason="rate-limit" onToggleBrake={() => {}} />);
    expect(screen.getByRole("button", { name: /brake on/i })).toBeInTheDocument();
    expect(screen.getByText(/rate-limit/)).toBeInTheDocument();
  });

  it("calls onNewProject when 'New project' is clicked", () => {
    const handler = vi.fn();
    render(<Topbar activeProject={null} onNewProject={handler} usage={null} brakeOn={false} onToggleBrake={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /new project/i }));
    expect(handler).toHaveBeenCalled();
  });
});
