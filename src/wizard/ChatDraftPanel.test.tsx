import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ChatDraftPanel } from "./ChatDraftPanel";
import { addTeam, emptyDraft } from "./draft";
import type { DraftPipeline } from "../ipc/pipeline";

const designSessionTurnMock = vi.fn();
const bestEffortValidateMock = vi.fn().mockResolvedValue([]);
vi.mock("../ipc/pipeline", () => ({
  designSessionTurn: (...a: unknown[]) => designSessionTurnMock(...a),
  bestEffortValidate: (...a: unknown[]) => bestEffortValidateMock(...a),
}));

describe("ChatDraftPanel", () => {
  beforeEach(() => {
    designSessionTurnMock.mockReset();
    bestEffortValidateMock.mockReset();
    bestEffortValidateMock.mockResolvedValue([]);
  });

  it("sends a turn with the current draft and applies updated_draft", async () => {
    const draft = emptyDraft();
    const updated = { ...draft, teams: [{ id: "research", name: "Research", prompt_body: "", runner: { kind: "claude-cli", model: "m", effort: { mode: "standard" } }, scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { min: 1, max: 1 } }] };
    designSessionTurnMock.mockResolvedValueOnce({ reply_text: "added research", updated_draft: updated, issues: [] });
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

    await waitFor(() => expect(designSessionTurnMock).toHaveBeenCalledWith("s1", "teams", draft, "add research"));
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

  it("shows the issues returned by a chat turn", async () => {
    designSessionTurnMock.mockResolvedValueOnce({
      reply_text: "added",
      updated_draft: emptyDraft(),
      issues: ["draft has no teams yet"],
    });
    render(
      <ChatDraftPanel
        sessionId="s1"
        step="teams"
        draft={emptyDraft()}
        onDraftChange={() => {}}
        renderDraft={() => <div />}
      />,
    );
    fireEvent.change(screen.getByPlaceholderText(/refine this step/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: "send" }));
    expect(await screen.findByText(/draft has no teams yet/i)).toBeInTheDocument();
  });

  it("re-fetches issues from the backend when the draft is edited manually", async () => {
    bestEffortValidateMock.mockResolvedValueOnce(["team 'research' has no prompt yet"]);
    let captured: ((d: DraftPipeline) => void) | null = null;
    render(
      <ChatDraftPanel
        sessionId="s1"
        step="prompts"
        draft={addTeam(emptyDraft(), "research", "Research")}
        onDraftChange={() => {}}
        renderDraft={(_d, onChange) => {
          captured = onChange;
          return <div />;
        }}
      />,
    );
    captured!(addTeam(emptyDraft(), "research", "Research"));
    expect(await screen.findByText(/has no prompt yet/i)).toBeInTheDocument();
    expect(bestEffortValidateMock).toHaveBeenCalled();
  });
});
