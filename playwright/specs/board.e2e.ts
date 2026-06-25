import { test, expect } from "@playwright/test";
import { seedBeforeMount } from "./helpers";
import { demoPipeline, type MockState } from "../mock/backend";

/// Spec 2 (S5 Task 4) — board:
/// seed a project + an active run with tasks across stages + per-stage occupancy →
/// assert the team lane store "n/cap" + pool indicators render → open a card → the
/// CardDrawer shows.
test("board renders lane occupancy/pool indicators and opens a card", async ({ page }) => {
  const projId = "proj-board";
  const root = "/mock/board";
  const runId = "run-board";

  const state: Partial<MockState> = {
    projects: [
      {
        id: projId,
        name: "Board project",
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
        { stage: "writer", occupancy: 2, capacity: 8 },
        { stage: "reviewer", occupancy: 1, capacity: 4 },
      ],
    },
    tasks: [
      { id: "task-w1", project_id: projId, pipeline: "demo", topic: "Draft intro", target_repo: null, target_scope: null, current_stage: "writer", state: "running", attempts: 0, parent_artifact: null, review_artifact: null, created_at: 11, updated_at: 11, run_id: runId, item_key: "intro" },
      { id: "task-w2", project_id: projId, pipeline: "demo", topic: "Draft body", target_repo: null, target_scope: null, current_stage: "writer", state: "queued", attempts: 0, parent_artifact: null, review_artifact: null, created_at: 12, updated_at: 12, run_id: runId, item_key: "body" },
      { id: "task-r1", project_id: projId, pipeline: "demo", topic: "Review intro", target_repo: null, target_scope: null, current_stage: "reviewer", state: "gated", attempts: 1, parent_artifact: null, review_artifact: "artifacts/intro.md", created_at: 13, updated_at: 13, run_id: runId, item_key: "intro" },
    ],
  };

  await seedBeforeMount(page, state);
  await page.goto("/");

  // The board is the default view. The writer lane shows its store + pool readouts.
  // The board renders "store n/cap" inside a progressbar labelled "<lane> store".
  const writerStore = page.getByRole("progressbar", { name: "Writer store" });
  await expect(writerStore).toBeVisible();
  await expect(writerStore).toHaveText(/store 2\/8/);
  await expect(writerStore).toHaveAttribute("aria-valuetext", "2 of 8");

  const reviewerStore = page.getByRole("progressbar", { name: "Reviewer store" });
  await expect(reviewerStore).toHaveText(/store 1\/4/);

  // Pool indicator: one running work-item at writer (busy 1) over the team max (3).
  await expect(page.getByLabel("Writer workers 1 of 3 busy")).toHaveText(/pool 1\/3/);

  // Three cards across the two team lanes are on the board.
  await expect(page.locator(".abp-card")).toHaveCount(3);

  // Open a card → the CardDrawer (a dialog) shows the task id + its tabs. The card
  // is labelled by item_key ("intro") and shows its task id ("task-w1").
  await page.locator(".abp-card", { hasText: "task-w1" }).click();
  const drawer = page.getByRole("dialog");
  await expect(drawer).toBeVisible();
  await expect(drawer.getByText("task-w1")).toBeVisible();
  await expect(drawer.getByRole("tab", { name: /^artifact/ })).toBeVisible();
  await expect(drawer.getByRole("tab", { name: /^history/ })).toBeVisible();
});
