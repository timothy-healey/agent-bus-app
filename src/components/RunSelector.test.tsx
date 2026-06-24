import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { RunSelector, runOptionLabel } from "./RunSelector";
import type { Run } from "../ipc/runtime";

const run = (id: string, completed = false): Run => ({
  id, pipeline: "p", project_id: "proj", generator_dry: false, completed, created_at: 0,
});

describe("RunSelector", () => {
  it("shows an empty hint and a Start button when there are no runs", () => {
    render(<RunSelector runs={[]} selectedRun={null} activeRun={null} onSelect={() => {}} onStartRun={() => {}} />);
    expect(screen.getByText(/no runs yet/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /start run/i })).toBeInTheDocument();
    // no listbox when there are no runs
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
  });

  it("lists the runs and marks the active one", () => {
    const runs = [run("R-aaaaaaaa"), run("R-bbbbbbbb", true)];
    render(<RunSelector runs={runs} selectedRun={runs[0]} activeRun={runs[0]} onSelect={() => {}} onStartRun={() => {}} />);
    const select = screen.getByRole("combobox", { name: /run/i }) as HTMLSelectElement;
    expect(select.value).toBe("R-aaaaaaaa");
    expect(screen.getByText(/aaaaaaaa · active/i)).toBeInTheDocument();
    expect(screen.getByText(/bbbbbbbb · completed/i)).toBeInTheDocument();
  });

  it("calls onSelect with the chosen run id", () => {
    const runs = [run("R-aaaaaaaa"), run("R-bbbbbbbb")];
    const onSelect = vi.fn();
    render(<RunSelector runs={runs} selectedRun={runs[0]} activeRun={runs[0]} onSelect={onSelect} onStartRun={() => {}} />);
    fireEvent.change(screen.getByRole("combobox", { name: /run/i }), { target: { value: "R-bbbbbbbb" } });
    expect(onSelect).toHaveBeenCalledWith("R-bbbbbbbb");
  });

  it("calls onStartRun when Start is clicked", () => {
    const onStartRun = vi.fn();
    render(<RunSelector runs={[]} selectedRun={null} activeRun={null} onSelect={() => {}} onStartRun={onStartRun} />);
    fireEvent.click(screen.getByRole("button", { name: /start run/i }));
    expect(onStartRun).toHaveBeenCalled();
  });

  it("disables Start and shows progress while starting", () => {
    render(<RunSelector runs={[]} selectedRun={null} activeRun={null} onSelect={() => {}} onStartRun={() => {}} starting />);
    const btn = screen.getByRole("button", { name: /starting/i }) as HTMLButtonElement;
    expect(btn).toBeDisabled();
  });

  it("shows a loading hint (not the empty state) during the initial fetch", () => {
    render(<RunSelector runs={[]} selectedRun={null} activeRun={null} onSelect={() => {}} onStartRun={() => {}} loading />);
    expect(screen.getByText(/loading runs/i)).toBeInTheDocument();
    expect(screen.queryByText(/no runs yet/i)).not.toBeInTheDocument();
  });

  it("runOptionLabel reflects active / running / completed", () => {
    expect(runOptionLabel(run("R-12345678"), true)).toMatch(/active/);
    expect(runOptionLabel(run("R-12345678"), false)).toMatch(/running/);
    expect(runOptionLabel(run("R-12345678", true), false)).toMatch(/completed/);
  });
});
