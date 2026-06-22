import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ProjectWizard } from "./ProjectWizard";

vi.mock("../ipc/workspace", () => ({
  createProject: vi.fn(),
}));

import { createProject } from "../ipc/workspace";

describe("ProjectWizard", () => {
  beforeEach(() => {
    (createProject as ReturnType<typeof vi.fn>).mockReset();
  });

  it("renders when open=true", () => {
    render(<ProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByLabelText(/project name/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/root path/i)).toBeInTheDocument();
  });

  it("does not render when open=false", () => {
    render(<ProjectWizard open={false} onClose={() => {}} onCreated={() => {}} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("calls createProject + onCreated on submit", async () => {
    const created = {
      id: "proj-x",
      name: "New",
      root_path: "/tmp/n",
      active_pipeline_id: null,
      created_at: 1,
      updated_at: 1,
    };
    (createProject as ReturnType<typeof vi.fn>).mockResolvedValueOnce(created);

    const onCreated = vi.fn();
    render(<ProjectWizard open={true} onClose={() => {}} onCreated={onCreated} />);

    fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "New" } });
    fireEvent.change(screen.getByLabelText(/root path/i), { target: { value: "/tmp/n" } });
    fireEvent.click(screen.getByRole("button", { name: /create project/i }));

    await waitFor(() => {
      expect(createProject).toHaveBeenCalledWith({ name: "New", root_path: "/tmp/n" });
      expect(onCreated).toHaveBeenCalledWith(created);
    });
  });

  it("disables submit when fields empty", () => {
    render(<ProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    const button = screen.getByRole("button", { name: /create project/i });
    expect(button).toBeDisabled();
  });
});
