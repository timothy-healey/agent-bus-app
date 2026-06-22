import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { ProjectList } from "./ProjectList";

vi.mock("../ipc/workspace", () => ({
  listProjects: vi.fn().mockResolvedValue([
    {
      id: "proj-1",
      name: "Splose DDD",
      root_path: "/Users/tim/splose",
      active_pipeline_id: null,
      created_at: 1700000000,
      updated_at: 1700000000,
    },
    {
      id: "proj-2",
      name: "Side project",
      root_path: "/Users/tim/side",
      active_pipeline_id: "ddd-spec-plan-impl",
      created_at: 1700000100,
      updated_at: 1700000100,
    },
  ]),
}));

describe("ProjectList", () => {
  it("renders the projects returned by listProjects", async () => {
    render(<ProjectList />);
    await waitFor(() => {
      expect(screen.getByText("Splose DDD")).toBeInTheDocument();
      expect(screen.getByText("Side project")).toBeInTheDocument();
    });
  });

  it("shows the root_path for each project", async () => {
    render(<ProjectList />);
    await waitFor(() => {
      expect(screen.getByText("/Users/tim/splose")).toBeInTheDocument();
    });
  });

  it("shows empty state when no projects exist", async () => {
    const { listProjects } = await import("../ipc/workspace");
    (listProjects as ReturnType<typeof vi.fn>).mockResolvedValueOnce([]);

    render(<ProjectList />);
    await waitFor(() => {
      expect(screen.getByText(/no projects yet/i)).toBeInTheDocument();
    });
  });
});
