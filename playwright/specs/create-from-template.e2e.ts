import { test, expect } from "@playwright/test";
import { seedBeforeMount, readState } from "./helpers";
import { demoDraft } from "../mock/backend";

/// Spec 1 (S5 Task 3) — create-from-template:
/// open the new-project flow → pick a seed template → the canvas renders the seeded
/// graph (team nodes + store nodes + edges) → Review → Create → land on the
/// run-scoped board (the mock's create_project_from_draft adds the project +
/// registers its pipeline, and the app selects the new project).
test("create from a seed template lands on the run-scoped board", async ({ page }) => {
  // No projects yet → the app shows the empty state; the topbar offers New project.
  await seedBeforeMount(page, {
    projects: [],
    seedTemplates: [
      { id: "writer-reviewer", name: "Writer + Reviewer", description: "writer hands off to a reviewer" },
    ],
    seedDrafts: { "writer-reviewer": demoDraft() },
  });
  await page.goto("/");

  // Open the wizard from the topbar.
  await page.getByRole("button", { name: "New project" }).click();

  // Basics: name + root are required before the template buttons enable.
  await page.getByLabel("Project name").fill("Acme docs");
  await page.getByRole("textbox", { name: "Root path" }).fill("/mock/acme");

  // Start from the bundled template (the A2 kickoff path — no live LLM).
  await page.getByRole("button", { name: "Writer + Reviewer" }).click();

  // Canvas step: the seeded graph renders. The PipelineCanvas tags nodes with
  // data-node-kind; the draft has two teams (writer/reviewer) + a gate + an
  // escalation, and the canvas projects an input store node for the reviewer.
  const canvas = page.getByTestId("pipeline-canvas");
  await expect(canvas).toBeVisible();
  await expect(canvas.locator('[data-node-kind="team"]')).toHaveCount(2);
  await expect(canvas.locator('[data-node-kind="store"]').first()).toBeVisible();
  // At least one edge is drawn (the writer → reviewer forward flow, via the store).
  await expect(canvas.locator(".react-flow__edge").first()).toBeVisible();

  // Continue → Review.
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByRole("heading", { name: "Prompt files" })).toBeVisible();

  // Create the project. The mock adds it + registers a resolved pipeline.
  await page.getByRole("button", { name: "Create project" }).click();

  // The wizard closes and we land on the board for the new project: the board's
  // "no run scoped" hint confirms a project + pipeline are active (lanes need a run).
  await expect(page.getByText("no run scoped. start a run to populate the board.")).toBeVisible();

  // The run selector (board-only chrome) is present, and the project is selected.
  await expect(page.getByRole("button", { name: "Start run" })).toBeVisible();

  // The mock mutated: the new project exists in state.
  const state = await readState(page);
  expect(state?.projects.map((p) => p.name)).toContain("Acme docs");
});
