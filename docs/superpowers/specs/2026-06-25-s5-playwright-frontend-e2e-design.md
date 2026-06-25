# Spec — S5: Local Playwright frontend E2E (runs on macOS)

*Design doc. Brainstormed 2026-06-25. Promotes backlog **S5** (investigate runnable E2E, revisit S4). Scope of THIS spec: the local, macOS-runnable half — Playwright over the Vite build with the Tauri IPC mocked. The packaged-app E2E on Linux/Windows CI (S4's WebDriver harness) and proving the live worker→`claude` path are documented as a follow-on, not built here.*

## Why this exists

S4's `tauri-driver` harness **can't run on macOS** (WKWebView exposes no WebDriver), so today there is **no automated test that drives the real app on the dev machine** — only unit/component tests + structural-only live paths. This adds a Playwright suite that runs the **real React app** in a plain browser against a **mocked Tauri IPC**, giving fast, local, end-to-end coverage of the actual UI flows (create → canvas → board → recovery). It does not need the Rust backend or a live model.

## Decisions (from the brainstorm)

1. **Scope: the local half now.** Playwright over the Vite build with the Tauri IPC mocked. S4's packaged-app CI path + the live-`claude` proof are a documented follow-on.
2. **IPC mock: Vite alias → a mock module.** A dedicated E2E Vite build aliases `@tauri-apps/api/core` → `mock-core` and `@tauri-apps/api/event` → `mock-event`; the real `ipc/*.ts` wrappers run against a hand-written, typed, scriptable mock (stable across Tauri versions). (Rejected: the version-fragile `__TAURI_INTERNALS__` global shim; the broad DI refactor.)
3. **Fenced like S4.** Own dep island, excluded from `vitest`/`tsc`/`bun run build`, install-on-demand — the default toolchain is untouched.

## Architecture

```
playwright/                         (self-contained, install-on-demand)
  package.json                      @playwright/test (dep island)
  tsconfig.json                     own; root tsconfig EXCLUDES this dir
  playwright.config.ts              webServer = vite (E2E mode) + Chromium project
  mock/
    tauri-core.ts                   mock `invoke(cmd, args)` over a command-handler map
    tauri-event.ts                  mock `listen/emit` over an event bus
    backend.ts                      in-memory fake state (projects/pipelines/tasks/runs/invocations) + the command handlers + a window.__E2E__ seed/override hook
  specs/
    create-from-template.e2e.ts
    board.e2e.ts
    needs-human-recovery.e2e.ts
  README.md                         how to run; what it does/doesn't cover
vite.config.e2e.ts (or an `--mode e2e`)  aliases @tauri-apps/api/{core,event} → playwright/mock/*
```

### The E2E build
A dedicated Vite config (or `--mode e2e` over the existing config) that adds `resolve.alias` for `@tauri-apps/api/core` → `playwright/mock/tauri-core` and `@tauri-apps/api/event` → `playwright/mock/tauri-event`. Everything else is the real app. Playwright's `webServer` runs this build (dev or preview) and Chromium loads it.

### The mock (a tiny JS fake backend)
- **`tauri-core.invoke(cmd, args)`** dispatches to a **command-handler map** backed by **mutable in-memory state** (`backend.ts`): seeded projects/pipelines/tasks/runs/invocations, and handlers for the OHS commands the UI actually calls — at minimum `workspace_list_projects`, `list_seed_templates`/`seed_template`, `create_project_from_draft`, `best_effort_validate`, `list_tasks`, `list_runs`, `run_store_occupancy`, `start_run`, `list_invocations`, `retry_task`/`force_advance`/`abandon_task`/`accept_task`, `list_skills`, `usage_snapshot`, `brake_state`, `send_message`. Handlers **mutate** state (create adds a project; retry moves a task) so the UI reflects real transitions. Unknown commands return a benign default + a console warning (so a new command surfaces, doesn't crash).
- **`tauri-event.listen(event, cb)` + `emit(event, payload)`** over an event bus, so the UI's `task-changed`/`run-changed`/`usage-changed`/`task-log`/`conversation-delta` subscriptions work; handlers/tests fire events to drive refreshes.
- **`window.__E2E__`** hook: per-test seed (`__E2E__.seed({...})`) + per-command override (`__E2E__.on("retry_task", fn)`) + `__E2E__.emit(event, payload)`, set via Playwright `addInitScript`, so each spec scripts its own scenario without rebuilding.

### Headline specs (real React flows; LLM-free; backend-mocked)
1. **create-from-template** — open the new-project flow → pick a seed template → the canvas renders the seeded graph (teams/edges/store nodes) → Review → Create → land on the run-scoped board.
2. **board** — seed a run with tasks across stages → assert lane store-occupancy "n/cap" + pool indicators → open a card (drawer).
3. **needs-human-recovery** — seed a failure-escalated task + its `invocation_audit` rows → the card shows the L3 history panel + the failure reason + the state-aware actions → click **Retry** → the mock requeues the task at the failed stage + emits `task-changed` → the board updates.

## Out of scope / non-goals (documented follow-on)
- The **live worker→`claude`** path, the **real Rust backend**, and the **packaged app** — these remain S4's Linux/Windows CI WebDriver path + a live-`claude` run with a key. S5 (this spec) is browser-frontend-only.
- Wiring Playwright (or S4) into **CI** — a follow-on once the local suite is green.
- Visual-regression/screenshot diffing — not now (flow coverage only).

## Testing / verification
- The Playwright suite IS the test (`playwright/` `test:e2e`). It must **not** be collected by `bun vitest run`, `tsc --noEmit`, or `bun run build` — verify those stay green and untouched (the fencing). A `playwright/README.md` documents `npx playwright install` + how to run, and the macOS-runs-here / live-path-elsewhere split.
- Because the mock is hand-written, a small **contract risk** exists (mock command shapes vs the real OHS): note it, and keep the mock's response shapes mirroring the TS IPC types (`ipc/*.ts`) so drift is caught at `tsc` within the playwright tsconfig.

## Relationship to other items
- **Supersedes/extends S4** (which stays the packaged-app CI path).
- The **live worker→`claude`** proof is explicitly deferred to the follow-on (CI packaged-app or a manual operator run) — S5-local can't exercise it (no backend).
