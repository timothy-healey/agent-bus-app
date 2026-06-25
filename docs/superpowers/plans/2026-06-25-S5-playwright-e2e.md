# S5 — Local Playwright frontend E2E

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Design source: `docs/superpowers/specs/2026-06-25-s5-playwright-frontend-e2e-design.md` (read first). Local, macOS-runnable Playwright suite over the Vite build with the Tauri IPC mocked. FENCED from the default toolchain (like S4's `e2e/`).

**Goal:** A Playwright suite that drives the real React app in Chromium against a mocked Tauri IPC, covering three headline flows — and the existing `vitest`/`cargo`/`tsc`/`build` gates stay green and untouched.

**Architecture:** A self-contained `playwright/` dir (own dep island + tsconfig) + a dedicated E2E Vite build that aliases `@tauri-apps/api/{core,event}` to a hand-written, scriptable mock backend (in-memory state + command-handler map + event bus + `window.__E2E__` hook). Playwright's `webServer` serves the E2E build; specs script scenarios via `__E2E__`.

**Tech Stack:** `@playwright/test`, Vite (E2E mode/alias), TypeScript.

---

## Tasks

- [ ] **Task 1 — Scaffold + fence.** Create `playwright/` with `package.json` (dep island: `@playwright/test`), `tsconfig.json`, `playwright.config.ts` (Chromium project; `webServer` runs the E2E Vite build). Add the **E2E Vite alias**: a `vite.config.e2e.ts` (or `--mode e2e` branch in the existing config) aliasing `@tauri-apps/api/core` → `playwright/mock/tauri-core` and `@tauri-apps/api/event` → `playwright/mock/tauri-event`. **Fence:** root `tsconfig.json` `exclude` adds `playwright/`; confirm `vite.config.ts` `test.exclude` (vitest) + `tsc`/`bun run build` never collect `playwright/`. Add `npx playwright install chromium`. Commit.
- [ ] **Task 2 — Mock backend.** `playwright/mock/backend.ts` — mutable in-memory state (projects, seed templates, draft→pipeline, tasks, runs, store occupancy, invocations, skills, usage, brake) + a `invoke(cmd,args)` command-handler map covering at least: `workspace_list_projects`, `list_seed_templates`, `seed_template`, `best_effort_validate`, `create_project_from_draft`, `activate_project`, `list_tasks`, `list_runs`, `run_store_occupancy`, `start_run`, `list_invocations`, `retry_task`, `force_advance`, `abandon_task`, `accept_task`, `list_skills`, `usage_snapshot`, `brake_state`, `send_message`. Handlers MUTATE state. Unknown command → benign default + `console.warn`. `tauri-core.ts` (re-exports `invoke`), `tauri-event.ts` (`listen`/`emit` over an event bus). `window.__E2E__` = `{ seed(state), on(cmd, fn), emit(event, payload) }` (installed via the mock module reading a global the spec sets through `addInitScript`). Mirror the response SHAPES to the `ipc/*.ts` TS types so the playwright tsconfig catches drift. Commit.
- [ ] **Task 3 — Spec: create-from-template.** Open the new-project flow → pick a seed template → assert the canvas renders the seeded graph (team nodes + edges + store nodes) → Review → Create → assert landing on the run-scoped board (the mock's `create_project_from_draft` adds the project + selects it). Commit.
- [ ] **Task 4 — Spec: board.** Seed (via `__E2E__.seed`) a project + an active run with tasks across stages + occupancy → assert lane store "n/cap" + pool indicators render → open a card → the drawer shows. Commit.
- [ ] **Task 5 — Spec: needs-human-recovery.** Seed a failure-escalated task + its `list_invocations` rows → open the card → assert the L3 history panel + the failure reason + the state-aware actions (Retry / Approve&advance / Abandon) → click **Retry** → the mock requeues the task (mutates state) + `emit("task-changed")` → assert the board reflects the move. Commit.
- [ ] **Task 6 — README + fencing verification.** `playwright/README.md` (install + run; macOS-runs-here vs the live-path-elsewhere split; the hand-written-mock contract caveat). Verify the default gates are untouched + green AND the Playwright suite passes. Commit.

## Verification gates
- **Default toolchain stays green + does NOT collect `playwright/`:** `cd src-tauri && cargo test --workspace`; `cd /Users/tim/projects/agent-bus-app && npx vitest run`; `npx tsc --noEmit`; `bun run build`.
- **The new suite runs (macOS):** `cd playwright && npx playwright install chromium && npx playwright test` — green.
- Tag: `git tag plan-S5`.

## Spec coverage
- Fenced local Playwright harness over the E2E Vite alias build → Task 1. ✓
- Scriptable mock backend (invoke map + event bus + `__E2E__`) → Task 2. ✓
- Three headline flow specs → Tasks 3–5. ✓
- Default toolchain untouched (fencing) → Tasks 1,6. ✓
- Out of scope (live claude / packaged app / CI) documented → Task 6 README. ✓

## Constraints
Local commits on `main`, NEVER push. Commit per task. The `playwright/` dir is install-on-demand and MUST stay out of the root `vitest`/`tsc`/`build`/`cargo` paths (verify). Mock response shapes mirror `ipc/*.ts` types (drift caught by the playwright tsconfig). Browser-frontend-only — no Rust backend, no live `claude`.
