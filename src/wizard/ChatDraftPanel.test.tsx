import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ChatDraftPanel } from "./ChatDraftPanel";
import { emptyDraft } from "./draft";

const turnMock = vi.fn();
vi.mock("../ipc/pipeline", () => ({
  designSessionTurn: (...a: unknown[]) => turnMock(...a),
}));

describe("ChatDraftPanel", () => {
  beforeEach(() => turnMock.mockReset());

  it("sends a turn with the current draft and applies updated_draft", async () => {
    const draft = emptyDraft();
    const updated = { ...draft, teams: [{ id: "research", name: "Research", prompt_body: "", runner: { kind: "claude-cli", model: "m", effort: { mode: "standard" } }, scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { default: 1, max: 1 } }] };
    turnMock.mockResolvedValueOnce({ reply_text: "added research", updated_draft: updated });
    const onDraftChange = vi.fn();

    render(
      <ChatDraftPanel
        sessionId="s1"
        step="teams"
        draft={draft}
        onDraftChange={onDraftChange}
        renderDraft={(d) => <div data-testid="draft-view">{d.teams.length} teams</div>}
      />,
    );
    fireEvent.change(screen.getByPlaceholderText(/refine/i), { target: { value: "add research" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(turnMock).toHaveBeenCalledWith("s1", "teams", draft, "add research"));
    await waitFor(() => expect(onDraftChange).toHaveBeenCalledWith(updated));
    expect(await screen.findByText("added research")).toBeInTheDocument();
  });

  it("renders the draft slot via renderDraft", () => {
    render(
      <ChatDraftPanel
        sessionId="s1"
        step="teams"
        draft={emptyDraft()}
        onDraftChange={() => {}}
        renderDraft={(d) => <div data-testid="draft-view">{d.teams.length} teams</div>}
      />,
    );
    expect(screen.getByTestId("draft-view")).toHaveTextContent("0 teams");
  });
});
