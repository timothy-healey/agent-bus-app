# Playwright frontend E2E (mocked Tauri IPC) — S5

A local, **macOS-runnable** Playwright suite that drives the **real React app** in
Chromium against a **hand-written, mocked Tauri IPC**. It covers three headline,
LLM-free flows end-to-end through the genuine UI:

1. **create-from-template** — open the new-project wizard → pick a seed template →
   the canvas renders the seeded graph (team + store nodes + edges) → Review →
   Create → land on the run-scoped board.
2. **board** — a project + an active run with tasks across stages + per-stage
   occupancy → the lane store `n/cap` + pool `busy/max` indicators render → open a
   card (the CardDrawer).
3. **needs-human-recovery** — a failure-escalated task + its invocation audit rows
   → the card's L3 history panel + the failure reason + the state-aware recovery
   actions → click **Retry** → the mock requeues the task + emits `task-changed` →
   the board updates.

No Rust backend, no live `claude` — just the React frontend over the mock.

---

## How it works

```
playwright/                         self-contained, install-on-demand
  package.json                      @playwright/test (+ @types/node) dep island
  tsconfig.json                     own; the root tsconfig EXCLUDES this dir
  playwright.config.ts              Chromium project + a webServer that builds/serves the E2E build
  mock/
    backend.ts                      in-memory state + invoke(cmd,args) handler map + event bus + window.__E2E__
    tauri-core.ts                   re-exports the mock invoke (aliased for @tauri-apps/api/core)
    tauri-event.ts                  mock listen/emit + Event envelope (aliased for @tauri-apps/api/event)
    tauri-dialog.ts                 benign stub for @tauri-apps/plugin-dialog
  specs/
    create-from-template.e2e.ts
    board.e2e.ts
    needs-human-recovery.e2e.ts
    helpers.ts                      seedBeforeMount / emitEvent / readState
../vite.config.e2e.ts               aliases @tauri-apps/api/{core,event} (+ plugin-dialog) → playwright/mock/*
```

**The E2E build** (`../vite.config.e2e.ts`) is the real app with one change:
`resolve.alias` points the two Tauri IPC entry points at the mock. The real
`src/ipc/*.ts` wrappers run unchanged on top, so the flows exercise the genuine
component tree, hooks, and IPC-wrapper code. Playwright's `webServer` runs
`vite build --config vite.config.e2e.ts && vite preview` and Chromium loads it on
port `1430` (the dev server's `1420` is left free).

**The mock** (`mock/backend.ts`) is mutable in-memory state + an `invoke(cmd,args)`
command-handler map. Handlers **mutate** state — `create_project_from_draft` adds
the project (and registers its resolved pipeline), `start_run` creates a run and
emits `run-changed`/`task-changed`, `retry_task` requeues a task and emits
`task-changed` — so the UI reflects real transitions. `listen`/`emit` ride an
event bus. An **unknown command** returns `null` + a `console.warn` (a newly-added
command surfaces during the run instead of crashing the UI).

**Per-test scripting** via `window.__E2E__ = { seed, on, emit, getState }`:

- `seedBeforeMount(page, state)` stashes a seed payload on `window.__E2E_SEED__`
  via `addInitScript` (runs **before** the app bundle); the mock applies it on
  load. This is the deterministic path — it beats the app's first data fetches.
- `emitEvent(page, event, payload)` fires a bus event from the test (post-mount).
- `readState(page)` reads the live mock state back to assert handler mutations.

---

## Run it (macOS or any desktop OS with Chromium)

From `playwright/`:

```bash
npm install                  # the dep island (@playwright/test + @types/node)
npx playwright install chromium   # one-time browser download
npx playwright test          # build the E2E app, serve it, run the suite
```

The `webServer` builds + serves the E2E app automatically; you do **not** start
Vite yourself. For a visible browser: `npx playwright test --headed`. Locally the
server is reused across reruns (`reuseExistingServer`).

---

## macOS runs here; the live path runs elsewhere

This is the **local half** of the E2E story (backlog S5). The split:

| Harness | What it drives | macOS? |
|---|---|---|
| **`playwright/` (this, S5)** | the **real React frontend** in Chromium over a **mocked** IPC | ✅ yes |
| `e2e/` (S4, WebDriver) | the **packaged Tauri app** via `tauri-driver` | ❌ Linux/Windows only |

`tauri-driver` exposes no automation surface on macOS (WKWebView has no
WebDriver), so S4 can never run on the dev machine. S5 fills that gap for the
**frontend** flows — but it deliberately does **not** exercise:

- the **live worker → `claude`** path (needs a real model + key),
- the **real Rust backend** (the OHS commands, the engine, persistence),
- the **packaged app** / native shell.

Those stay S4's Linux/Windows CI WebDriver path + a manual live-`claude` run, and
remain a documented follow-on (also: wiring either suite into CI;
visual-regression diffing — not done here, flow coverage only).

## The hand-written-mock contract caveat

`mock/backend.ts` is a **fake**, not the real OHS. A green suite proves the UI
flow against the mock's response **shapes**, not against the live Rust backend, so
a small **contract risk** exists (mock shapes drifting from the real commands). It
is mitigated, not eliminated: every mock response is typed against the app's own
`src/ipc/*.ts` types, so `tsc` in this dir (`cd playwright && npx tsc --noEmit`)
flags drift the moment an IPC type changes. The real backend stays the authority;
the live contract is proven by S4 + the live-`claude` follow-on, not here.

---

## Fenced out of the default toolchain

This directory is **install-on-demand** and never touched by the repo's default
gates:

- **vitest** never collects it — `../vite.config.ts` `test.exclude` adds
  `playwright/**`, and the specs are `*.e2e.ts` (outside vitest's `{test,spec}`
  include) anyway.
- **`tsc --noEmit`** / **`bun run build`** never compile it — the root
  `../tsconfig.json` `include: ["src"]` + explicit `exclude: [... "playwright"]`.
- **`cargo`** is unaffected (it's a Rust workspace under `../src-tauri`).
- The dep island (`playwright/package.json`) keeps `@playwright/test` out of the
  repo's root install.
