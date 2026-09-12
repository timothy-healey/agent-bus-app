import { fileURLToPath } from "node:url";
import { test } from "@playwright/test";
import { seedBeforeMount } from "./helpers";
import type { MockState } from "../mock/backend";
import type { Pipeline, Team } from "../../src/ipc/pipeline";
import type { Task } from "../../src/ipc/runtime";

/// README screenshots — the real React app over the mocked IPC (same build the
/// E2E specs drive), seeded with a plausible mid-run DDD pipeline. Opt-in, so a
/// plain `npx playwright test` never rewrites the committed images:
///
///   SHOTS=1 npx playwright test specs/shots.e2e.ts
const OUT = fileURLToPath(new URL("../../docs/assets/screenshots", import.meta.url));

test.skip(!process.env.SHOTS, "set SHOTS=1 to regenerate the README screenshots");

const PROJ = "proj-orders";
const ROOT = "/Users/you/code/orders-service";
const RUN = "run-01";

const runner = { kind: "claude-cli" as const, model: "claude-opus-4-8", effort: { mode: "standard" as const } };

function producer(id: string, name: string, onApprove: string, capacity: number, extra: Partial<Team> = {}): Team {
  return {
    id, name, prompt: "", runner,
    scope: { reads: [], writes: ["artifacts/"], tools: [] },
    outputs: { on_approve: onApprove },
    workers: { min: 1, max: 4 },
    role: "producer",
    store: { capacity },
    ...extra,
  };
}

function reviewer(id: string, name: string, onApprove: string, reviseTo: string, capacity: number): Team {
  return {
    id, name, prompt: "", runner,
    scope: { reads: ["artifacts/"], writes: [], tools: [] },
    outputs: { on_approve: onApprove, on_revise: reviseTo, on_reject: "needs-human" },
    workers: { min: 1, max: 2 },
    role: "reviewer",
    store: { capacity },
  };
}

/// Mirrors the bundled `ddd-spec-plan-impl` seed template
/// (src-tauri/pipeline/src/seed_template.rs) so the shots show the real default.
function dddPipeline(): Pipeline {
  return {
    id: "ddd-spec-plan-impl",
    name: "DDD: Spec → Plan → Implement",
    description:
      "Research → spec → spec-review → plan → plan-review → implement → code-review → hand off to human.",
    schema_version: 3,
    teams: [
      producer("research", "Research", "spec-writers", 8),
      producer("spec-writers", "Spec Writers", "spec-reviewers", 6, { workers: { min: 1, max: 3 } }),
      reviewer("spec-reviewers", "Spec Reviewers", "gate-spec", "spec-writers", 4),
      producer("plan-writers", "Plan Writers", "plan-reviewers", 6, { workers: { min: 1, max: 3 } }),
      reviewer("plan-reviewers", "Plan Reviewers", "implementers", "plan-writers", 4),
      producer("implementers", "Implementers", "code-reviewers", 6, { workers: { min: 1, max: 4 } }),
      reviewer("code-reviewers", "Code Reviewers", "needs-human", "implementers", 4),
    ],
    gates: [{ id: "gate-spec", label: "Spec Approval (human)", downstream: "plan-writers" }],
    escalations: [{ id: "needs-human", triggers: ["attempts >= 3", "verdict == reject"] }],
    forks: [],
    joins: [],
  } as Pipeline;
}

let seq = 0;
function task(stage: string, state: Task["state"], itemKey: string, topic: string, over: Partial<Task> = {}): Task {
  seq += 1;
  return {
    id: `task-${String(seq).padStart(2, "0")}`,
    project_id: PROJ,
    pipeline: "ddd-spec-plan-impl",
    topic,
    target_repo: ROOT,
    target_scope: null,
    current_stage: stage,
    state,
    attempts: 0,
    parent_artifact: null,
    review_artifact: null,
    created_at: 100 + seq,
    updated_at: 200 + seq,
    run_id: RUN,
    item_key: itemKey,
    ...over,
  };
}

const TASKS: Task[] = [
  task("research", "done", "pricing", "Scan orders-service for pricing seams"),
  task("spec-writers", "running", "refunds", "Spec: refund policy as its own aggregate"),
  task("spec-writers", "queued", "shipping", "Spec: split shipping quotes from fulfilment"),
  task("spec-reviewers", "gated", "pricing", "Review spec: pricing aggregate boundary", { attempts: 1, review_artifact: "artifacts/pricing-spec.md" }),
  task("plan-writers", "running", "checkout", "Plan: extract checkout ACL", { attempts: 1 }),
  task("plan-reviewers", "revising", "inventory", "Vet plan: inventory read model", { attempts: 2, review_artifact: "artifacts/inventory-plan.md" }),
  task("implementers", "running", "catalog", "Implement: catalog value objects", { attempts: 1, worktree_path: `${ROOT}/.worktrees/catalog` }),
  task("implementers", "running", "payments", "Implement: payments anti-corruption layer", { attempts: 1, worktree_path: `${ROOT}/.worktrees/payments` }),
  task("implementers", "queued", "notifications", "Implement: notification outbox"),
  task("code-reviewers", "gated", "catalog", "Code review: catalog value objects", { attempts: 1, review_artifact: "artifacts/catalog.diff" }),
  task("needs-human", "needs_human", "legacy-import", "Legacy CSV import — declined twice", { attempts: 3 }),
];

const TOKENS: Record<string, number> = Object.fromEntries(
  TASKS.map((t, i) => [t.id, [18_400, 46_200, 0, 31_900, 52_700, 88_100, 124_600, 96_300, 0, 41_500, 73_800][i]]),
);

function state(): Partial<MockState> {
  return {
    projects: [{
      id: PROJ, name: "orders-service", root_path: ROOT, target_repo: ROOT,
      skill_sources: [], active_pipeline_id: "ddd-spec-plan-impl", created_at: 1, updated_at: 1,
    }],
    pipelinesByRoot: { [ROOT]: dddPipeline() },
    pipelineIdsByRoot: { [ROOT]: ["ddd-spec-plan-impl"] },
    runs: [{ id: RUN, pipeline: "ddd-spec-plan-impl", project_id: PROJ, generator_dry: false, completed: false, created_at: 10 }],
    occupancyByRun: {
      [RUN]: [
        { stage: "research", occupancy: 1, capacity: 8 },
        { stage: "spec-writers", occupancy: 2, capacity: 6 },
        { stage: "spec-reviewers", occupancy: 1, capacity: 4 },
        { stage: "plan-writers", occupancy: 1, capacity: 6 },
        { stage: "plan-reviewers", occupancy: 1, capacity: 4 },
        { stage: "implementers", occupancy: 3, capacity: 6 },
        { stage: "code-reviewers", occupancy: 1, capacity: 4 },
      ],
    },
    tasks: TASKS,
    invocationsByTask: {
      "task-10": [
        { invocation_id: "inv-1", team_id: "implementers", model: "claude-opus-4-8", attempts: 1, started_at: 1700, settled_at: 1760, outcome: "verdict:approve", input_tokens: 28_400, output_tokens: 13_100 },
        { invocation_id: "inv-2", team_id: "code-reviewers", model: "claude-opus-4-8", attempts: 1, started_at: 1780, settled_at: null, outcome: "", input_tokens: 0, output_tokens: 0 },
      ],
    },
    usage: {
      window_total: 594_000, window_budget: 1_000_000, window_pct: 0.594, band: "warn",
      burn_per_min: 2_140, window_secs: 18_000, reset_in_secs: 7_020, est_brake_at: null,
      by_team: [
        { team_id: "implementers", tokens: 309_000 },
        { team_id: "plan-writers", tokens: 141_000 },
        { team_id: "spec-writers", tokens: 92_000 },
        { team_id: "code-reviewers", tokens: 52_000 },
      ],
      tokens_by_task: TOKENS, braked: false, auto_meter_enabled: true,
    },
    brake: { on: false, reason: null },
    conversation: {
      project_id: PROJ,
      session_id: "sess-01",
      started_at: 1600,
      last_message_at: 1900,
      history_budget_tokens: 100_000,
      summary_of_prior_sessions: null,
      turns: [
        { role: "user", text: "implementers are lagging behind the plan reviewers — give them more workers", tool_calls: [], at: 1800 },
        {
          role: "assistant",
          text: "Scaled implementers from 2 to 4 workers. The plan-reviewers store is at 1/4, so the backpressure clears from here.",
          tool_calls: [{ request: { tool_name: "scale_team", args: { team: "implementers", max: 4 } }, result: { status: "ok", result: 4 } }],
          at: 1810,
        },
        { role: "user", text: "what is holding up the pricing spec?", tool_calls: [], at: 1880 },
        {
          role: "assistant",
          text: "It is waiting on you. spec-reviewers approved it into the Spec Approval gate 6 minutes ago — open the card to approve, revise, or decline.",
          tool_calls: [{ request: { tool_name: "list_tasks", args: { state: "gated" } }, result: { status: "ok", result: 2 } }],
          at: 1890,
        },
      ],
    },
  };
}

const ARTIFACT = `# Catalog value objects

Extracted \`Sku\`, \`Money\`, and \`Quantity\` out of the anaemic \`CatalogItem\` record and into the catalog bounded context as value objects that hold their own invariants.

## What changed

- \`Sku\` validates its format at construction — no more bare \`String\` skus crossing a context boundary.
- \`Money\` carries its currency and refuses mixed-currency arithmetic.
- \`Quantity\` is non-negative by construction, removing six defensive checks in ordering.

## Acceptance criteria

- No primitive \`sku\` / \`price_cents\` fields remain in \`catalog::\`
- Mixed-currency arithmetic is a compile error, not a runtime check
- 41 tests pass, 9 of them new
`;

function comment(id: string, anchor: string, note: string, at: number) {
  const offset = ARTIFACT.indexOf(anchor);
  return {
    id, task_id: "task-10", artifact_path: "artifacts/catalog.diff",
    anchor_text: anchor, anchor_offset: offset,
    note, kind: "inline" as const, created_at: at,
    status: "open" as const, effective_offset: offset,
  };
}

const COMMENTS = [
  comment("c-1", "refuses mixed-currency arithmetic",
    "Good — this is the invariant the ordering context kept re-checking by hand.", 1880),
  comment("c-2", "removing six defensive checks in ordering",
    "Name the six in the plan so the next reviewer can confirm they are all gone.", 1890),
];

async function boot(page: import("@playwright/test").Page) {
  await seedBeforeMount(page, state());
  await page.goto("/");
  await page.waitForTimeout(1200);
  await page.evaluate(([body, comments]) => {
    window.__E2E__?.on("read_artifact", () => body);
    window.__E2E__?.on("list_comments", () => comments);
    window.__E2E__?.on("reanchor_comments", () => comments);
  }, [ARTIFACT, COMMENTS] as const);
}

test.describe("wide", () => {
  test.use({ viewport: { width: 2400, height: 585 }, deviceScaleFactor: 2 });

  test("board", async ({ page }) => {
    await boot(page);
    await page.getByRole("button", { name: "collapse terminal" }).click();
    await page.waitForTimeout(600);
    await page.screenshot({ path: `${OUT}/board.png` });
  });

  test("board dark", async ({ page }) => {
    await boot(page);
    await page.getByRole("button", { name: /dark mode/i }).click();
    await page.waitForTimeout(400);
    await page.getByRole("button", { name: "collapse terminal" }).click();
    await page.waitForTimeout(600);
    await page.screenshot({ path: `${OUT}/board-dark.png` });
  });
});

test.describe("standard", () => {
  test.use({ viewport: { width: 1500, height: 830 }, deviceScaleFactor: 2 });

  test("review drawer", async ({ page }) => {
    await boot(page);
    await page.locator(".abp-card", { hasText: "Code review: catalog value objects" }).click();
    await page.waitForTimeout(1200);
    await page.screenshot({ path: `${OUT}/review.png` });
  });

  test("terminal", async ({ page }) => {
    await boot(page);
    // Drag the terminal's resize handle up so the whole exchange is in frame.
    const handle = page.getByLabel("resize terminal");
    const hb = await handle.boundingBox();
    if (hb) {
      await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
      await page.mouse.down();
      await page.mouse.move(hb.x + hb.width / 2, hb.y - 120, { steps: 12 });
      await page.mouse.up();
    }
    await page.waitForTimeout(600);
    const top = (await handle.boundingBox())!.y;
    const vp = page.viewportSize()!;
    await page.screenshot({
      path: `${OUT}/terminal.png`,
      clip: { x: 0, y: top, width: vp.width, height: vp.height - top },
    });
  });
});

test.describe("graph", () => {
  test.use({ viewport: { width: 2200, height: 900 }, deviceScaleFactor: 2 });

  test("pipeline graph", async ({ page }) => {
    await boot(page);
    await page.getByRole("tab", { name: /pipeline/i }).click();
    await page.waitForTimeout(1000);
    // The pipeline column is a fixed 760px reading measure, so the 9-node graph
    // scrolls inside it. Let it overflow for a detail shot of the whole flow.
    await page.addStyleTag({
      content: "[data-testid=pipeline-graph]{overflow:visible !important;width:max-content !important}",
    });
    await page.waitForTimeout(400);
    const graph = page.locator("[data-testid=pipeline-graph]");
    await graph.screenshot({ path: `${OUT}/pipeline.png` });
  });
});
