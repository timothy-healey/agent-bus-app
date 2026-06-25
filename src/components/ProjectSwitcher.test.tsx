import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ProjectSwitcher } from "./ProjectSwitcher";
import type { Project } from "../ipc/workspace";

function proj(id: string, name: string): Project {
  return {
    id, name, root_path: `/p/${id}`, target_repo: null, skill_sources: [],
    active_pipeline_id: null, created_at: 0, updated_at: 0,
  };
}

const projects = [proj("p1", "Alpha"), proj("p2", "Beta")];

describe("ProjectSwitcher (G13/G14)", () => {
  it("lists projects, marks the active one, and selects on change", () => {
    const onSelect = vi.fn();
    render(<ProjectSwitcher projects={projects} activeProjectId="p1" onSelect={onSelect} />);
    const select = screen.getByRole("combobox", { name: /project/i });
    expect(select).toHaveValue("p1");
    expect(screen.getByText("Alpha (active)")).toBeInTheDocument();
    fireEvent.change(select, { target: { value: "p2" } });
    expect(onSelect).toHaveBeenCalledWith("p2");
  });

  it("shows an empty state when there are no projects", () => {
    render(<ProjectSwitcher projects={[]} activeProjectId={null} />);
    expect(screen.getByText(/no projects yet/i)).toBeInTheDocument();
  });

  it("delete is guarded by an inline confirm before calling onDelete", async () => {
    const onDelete = vi.fn().mockResolvedValue(undefined);
    render(<ProjectSwitcher projects={projects} activeProjectId="p1" onSelect={() => {}} onDelete={onDelete} />);
    // first click reveals the confirm; nothing deleted yet.
    fireEvent.click(screen.getByRole("button", { name: /delete project alpha/i }));
    expect(onDelete).not.toHaveBeenCalled();
    expect(screen.getByText(/delete .*alpha/i)).toBeInTheDocument();
    // confirm deletes the active project.
    fireEvent.click(screen.getByRole("button", { name: /confirm delete alpha/i }));
    await waitFor(() => expect(onDelete).toHaveBeenCalledWith("p1"));
  });

  it("cancel dismisses the confirm without deleting", () => {
    const onDelete = vi.fn();
    render(<ProjectSwitcher projects={projects} activeProjectId="p1" onSelect={() => {}} onDelete={onDelete} />);
    fireEvent.click(screen.getByRole("button", { name: /delete project alpha/i }));
    fireEvent.click(screen.getByRole("button", { name: /^cancel$/i }));
    expect(onDelete).not.toHaveBeenCalled();
    // back to the un-confirmed affordance.
    expect(screen.getByRole("button", { name: /delete project alpha/i })).toBeInTheDocument();
  });

  it("a rejecting onDelete does not throw and still resets the confirm row", async () => {
    // Regression: onDelete used to be awaited with no catch → unhandled rejection
    // + stuck confirm. App's handler now surfaces the error; the switcher's
    // finally must still reset confirming.
    const onDelete = vi.fn().mockRejectedValue(new Error("FOREIGN KEY constraint failed"));
    render(<ProjectSwitcher projects={projects} activeProjectId="p1" onSelect={() => {}} onDelete={onDelete} />);
    fireEvent.click(screen.getByRole("button", { name: /delete project alpha/i }));
    fireEvent.click(screen.getByRole("button", { name: /confirm delete alpha/i }));
    await waitFor(() => expect(onDelete).toHaveBeenCalledWith("p1"));
    // confirm row reset → the plain delete affordance is back, no crash.
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /delete project alpha/i })).toBeInTheDocument(),
    );
  });

  it("hides the delete affordance when onDelete is omitted", () => {
    render(<ProjectSwitcher projects={projects} activeProjectId="p1" onSelect={() => {}} />);
    expect(screen.queryByRole("button", { name: /delete project/i })).not.toBeInTheDocument();
  });

  it("renders read-only (disabled select) when onSelect is omitted", () => {
    render(<ProjectSwitcher projects={projects} activeProjectId="p1" />);
    expect(screen.getByRole("combobox", { name: /project/i })).toBeDisabled();
  });
});
