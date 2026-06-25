import { test, expect } from "@playwright/test";
import { seedBeforeMount, readState } from "./helpers";
import { demoPipeline, type MockState } from "../mock/backend";

/// Spec 3 (S5 Task 5) — needs_human recovery:
/// seed a failure-escalated task + its list_invocations rows → open the card →
/// assert the L3 history panel + the failure reason line + the state-aware action
/// bar (Retry / Approve&advance / Abandon) → click Retry → the mock requeues the
/// task (mutates state to queued) + emits task-changed → the board reflects the move.
test("needs_human failure card shows L3 history + recovery actions; Retry requeues", async ({ page }) => {
  const projId = "proj-nh";
  const root = "/mock/nh";
  const runId = "run-nh";
  const taskId = "task-failed";

  const state: Partial<MockState> = {
    projects: [
      {
        id: projId,
        name: "Recovery project",
        root_path: root,
        target_repo: null,
        skill_sources: [],
        active_pipeline_id: "demo",
        created_at: 1,
        updated_at: 1,
      },
    ],
    pipelinesByRoot: { [root]: demoPipeline() },
    pipelineIdsByRoot: { [root]: ["demo"] },
    runs: [
      { id: runId, pipeline: "demo", project_id: projId, generator_dry: false, completed: false, created_at: 10 },
    ],
    occupancyByRun: {
      [runId]: [
        { stage: "writer", occupancy: 1, capacity: 8 },
        { stage: "reviewer", occupancy: 0, capacity: 4 },
      ],
    },
    tasks: [
      {
        id: taskId, project_id: projId, pipeline: "demo", topic: "Escalated work-item",
        target_repo: null, target_scope: null, current_stage: "writer", state: "needs_human",
        attempts: 3, parent_artifact: null, review_artifact: null, created_at: 11, updated_at: 11,
        run_id: runId, item_key: "escalated",
      },
    ],
    invocationsByTask: {
      [taskId]: [
        // newest-first; latest is an operational error → classified "failure".
        { invocation_id: "inv-3", team_id: "writer", model: "claude-opus-4-8", attempts: 3, started_at: 100, settled_at: 110, outcome: "error:rate_limited", input_tokens: 1200, output_tokens: 300 },
        { invocation_id: "inv-2", team_id: "writer", model: "claude-opus-4-8", attempts: 2, started_at: 80, settled_at: 90, outcome: "error:spawn", input_tokens: 800, output_tokens: 0 },
        { invocation_id: "inv-1", team_id: "writer", model: "claude-opus-4-8", attempts: 1, started_at: 60, settled_at: 70, outcome: "verdict:revise", input_tokens: 500, output_tokens: 200 },
      ],
    },
  };

  await seedBeforeMount(page, state);
  await page.goto("/");

  // The escalated card sits in the writer lane. Open it.
  await page.locator(".abp-card", { hasText: taskId }).click();
  const drawer = page.getByRole("dialog");
  await expect(drawer).toBeVisible();

  // The failure reason line shows (state-aware header), classified as a failure.
  const reason = drawer.getByTestId("reason-line");
  await expect(reason).toBeVisible();
  await expect(reason).toHaveAttribute("data-kind", "failure");
  await expect(reason).toContainText("needs you");

  // The action bar is in failure recovery mode → Retry / Approve&advance / Abandon.
  const actionBar = drawer.getByTestId("action-bar");
  await expect(actionBar).toHaveAttribute("data-mode", "needs_human:failure");
  await expect(drawer.getByRole("button", { name: "retry" })).toBeVisible();
  await expect(drawer.getByRole("button", { name: "approve and advance" })).toBeVisible();
  await expect(drawer.getByRole("button", { name: "abandon" })).toBeVisible();

  // The L3 history panel lists the three audit rows (tab badge shows the count).
  await drawer.getByRole("tab", { name: /^history/ }).click();
  const historyPanel = drawer.getByTestId("history-panel");
  await expect(historyPanel.getByRole("listitem")).toHaveCount(3);
  await expect(historyPanel).toContainText("rate limited");

  // Click Retry → the mock requeues the task (state → queued, attempts reset) and
  // emits task-changed; the board refetches.
  await drawer.getByRole("button", { name: "retry" }).click();

  // The mock mutated: the task is now queued with attempts reset.
  await expect.poll(async () => {
    const s = await readState(page);
    return s?.tasks.find((t) => t.id === taskId)?.state;
  }).toBe("queued");

  // The board reflects the move: the card is no longer needs_human, so the
  // failure reason line is gone and the action bar drops to read-only.
  await expect(drawer.getByTestId("reason-line")).toHaveCount(0);
  await expect(drawer.getByTestId("action-bar")).toHaveAttribute("data-mode", "read-only");
});
