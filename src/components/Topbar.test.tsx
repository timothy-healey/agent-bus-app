import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Topbar } from "./Topbar";

describe("Topbar", () => {
  it("shows the brand mark", () => {
    render(<Topbar activeProject={null} onNewProject={() => {}} />);
    expect(screen.getByText(/agent bus/i)).toBeInTheDocument();
  });

  it("shows 'No project' pill when activeProject is null", () => {
    render(<Topbar activeProject={null} onNewProject={() => {}} />);
    expect(screen.getByText(/no project/i)).toBeInTheDocument();
  });

  it("shows the project name when activeProject is set", () => {
    render(
      <Topbar
        activeProject={{
          id: "proj-1",
          name: "Splose",
          root_path: "/x",
          active_pipeline_id: null,
          created_at: 1,
          updated_at: 1,
        }}
        onNewProject={() => {}}
      />
    );
    expect(screen.getByText("Splose")).toBeInTheDocument();
  });

  it("calls onNewProject when 'New project' is clicked", () => {
    const handler = vi.fn();
    render(<Topbar activeProject={null} onNewProject={handler} />);
    fireEvent.click(screen.getByRole("button", { name: /new project/i }));
    expect(handler).toHaveBeenCalled();
  });
});
