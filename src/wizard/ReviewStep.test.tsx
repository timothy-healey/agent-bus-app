import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ReviewStep } from "./ReviewStep";
import { addTeam, emptyDraft, setPromptBody } from "./draft";

const createMock = vi.fn();
vi.mock("../ipc/pipeline", () => ({
  createProjectFromDraft: (...a: unknown[]) => createMock(...a),
}));

describe("ReviewStep", () => {
  beforeEach(() => createMock.mockReset());

  function draft() {
    let d = addTeam(emptyDraft(), "research", "Research");
    d = setPromptBody(d, "research", "investigate");
    return { ...d, name: "Demo" };
  }

  it("renders the assembled pipeline + the prompt files", () => {
    render(<ReviewStep basics={{ name: "Demo", root: "/p", description: "" }} draft={draft()} onCreated={() => {}} />);
    expect(screen.getByText("investigate")).toBeInTheDocument();
    expect(screen.getByText(/prompts\/research\.md/)).toBeInTheDocument();
  });

  it("Create calls createProjectFromDraft and onCreated on success", async () => {
    const created = { id: "proj-x", name: "Demo", root_path: "/p", active_pipeline_id: "demo", created_at: 0, updated_at: 0 };
    createMock.mockResolvedValueOnce(created);
    const onCreated = vi.fn();
    render(<ReviewStep basics={{ name: "Demo", root: "/p", description: "" }} draft={draft()} onCreated={onCreated} />);
    fireEvent.click(screen.getByRole("button", { name: /create/i }));
    await waitFor(() => expect(createMock).toHaveBeenCalledWith("Demo", "/p", expect.objectContaining({ name: "Demo" })));
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith(created));
  });

  it("shows the backend error when create fails (writes nothing)", async () => {
    createMock.mockRejectedValueOnce(new Error("team 'research' is unreachable"));
    render(<ReviewStep basics={{ name: "Demo", root: "/p", description: "" }} draft={draft()} onCreated={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /create/i }));
    expect(await screen.findByText(/unreachable/)).toBeInTheDocument();
  });
});
