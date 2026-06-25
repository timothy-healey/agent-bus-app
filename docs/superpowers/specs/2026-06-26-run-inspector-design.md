# Spec — Run Inspector: run-level observability + health

*Design doc. Brainstormed 2026-06-26. Operability/trust: a dedicated run-scoped view (+ a board health banner) that surfaces **what a run did** (per-invocation audit: outcome, cost, timing, the produced artifact) and **how it's doing / why it's stuck** (a health verdict + stall diagnosis + contextual recovery). Pure read projection over already-persisted data — no new aggregate, no log persistence. Closes roadmap "see what it did" (1) + "why did it stall/fail" (3).*

## Why this exists

You can't currently review a run unattended. `invocation_audit` (R3) records every invocation (outcome, tokens, timing) but has **no UI**; the live log is ephemeral (a settled card shows nothing); and nothing tells you *why* a run isn't progressing — a stall (a full store with no consumer), an error loop, or just completion all look the same from the board. To trust the app on a real repo while you're away, you need a surface that answers "what happened, how is it doing, and what do I do about it" from data that already exists.

Decided scope (from the brainstorm): **metadata + artifact only** — no persisting/replaying the full worker output/thinking stream (that stays running-only, as the live-worker-view spec deferred). This needs **no schema change**.

## Decisions

1. **Read-only projection.** A `run_report(run_id) → RunReport` OHS command in `runtime` assembles from `runs` + `invocation_audit` + `stores` + `tasks` + the generator-dry flag. No state mutation, no new aggregate, no migration.
2. **Health is a pure function.** `diagnose_run(input, now) → RunHealth` (injected `now`) classifies the run; trivially unit-testable, `runtime`-internal, Tauri-free.
3. **Dedicated "run" view + a shared board banner.** A new top-level view (ViewSwitcher entry `run`) renders the health banner + run rollup + audit table; the same `RunHealthBanner` component also renders on the board for at-a-glance status.
4. **Recovery reuses existing commands**, surfaced contextually by verdict (retry / force-advance / abandon / accept from H/L2 + brake Stop/Resume) — PLUS one small convenience, `retry_errored_in_run(run_id)`, since hunting individual errored items during a stall is the real pain.
5. **Cost basis matches LF34** — per-run total = sum of `input+output+cache_creation+cache_read` over the run's audit rows (the all-tokens basis the meter now uses).

## Architecture

### Backend (`runtime` crate)

**Audit read** — `InvocationAuditStore::list_for_run(run_id) → Vec<InvocationAuditRow>` (new; the store today only writes). Audit rows carry `task_id`; the report joins `task_id → tasks` for `item_key` + artifact path. A generator pass runs under a `gen:<run>:<stage>` `task_id` with **no `tasks` row** — handle gracefully (item_key from the id, no artifact).

**`run_report` command + DTOs:**
```rust
pub struct RunReport {
    pub run_id: String,
    pub started_at: i64,
    pub completed: bool,
    pub health: RunHealth,
    pub cost_tokens: u64,                 // all-tokens incl. cache, summed over the run's audit
    pub counts: ItemCounts,               // total/done/gated/needs_human/running; `errored` is a
                                          // SUBSET of needs_human (items whose latest audit outcome
                                          // is error:* — what retry_errored_in_run targets), not a
                                          // separate task state
    pub invocations: Vec<InvocationRow>,  // newest-first
}
pub struct InvocationRow {
    pub invocation_id: String, pub stage: String, pub item_key: String,
    pub model: String, pub attempts: u32,
    pub started_at: i64, pub settled_at: Option<i64>,
    pub outcome: Option<String>,          // "approve"|"revise"|"reject"|"error:<class>"|None(in-flight)
    pub tokens: u64,                       // in+out+cache for this invocation
    pub artifact_path: Option<String>,
}
pub enum RunHealthState { Running, Complete, NeedsHuman, StalledBackpressure, StalledIdle, ErrorLoop }
pub struct RunHealth { pub state: RunHealthState, pub reason: String }
```

**`diagnose_run`** — pure, precedence top-down (first match wins):
```
ErrorLoop            — ≥ ERROR_LOOP_MIN (3) error:* settles at one stage within RECENT_SECS (600)
                       with no `approve` settle at that stage in the same window
NeedsHuman           — ≥1 task in state gated|needs_human  → "N awaiting you at <stage>"
StalledBackpressure  — a store at occupancy==capacity AND no settle anywhere in the run for
                       STALL_NO_SETTLE_SECS (180) → "<stage> full (occ/cap), no settle 3m"
StalledIdle          — running_count==0 AND !generator_dry AND no settle for STALL_NO_SETTLE_SECS
                       → "idle, nothing progressing"
Complete             — generator_dry AND no resident items (queued|running|gated|revising|joining)
Running              — default → "active" (or "N in flight")
```
Input is the minimal current state (generator_dry, store occupancy/capacity, task stage+state, recent audit settle/outcome timestamps, in-flight count) + `now`. Thresholds are tunable heuristic constants in one place; the verdict is best-effort and documented as such.

**`retry_errored_in_run(run_id)`** — finds the run's tasks currently `needs_human` whose latest audit outcome is `error:*`, and re-queues each through the EXISTING retry path (`retry_task` logic — reset to `queued` at its stage, reset attempts). A thin loop over the existing per-item recovery; returns the count retried. (Genuine model failures that hit MAX_ATTEMPTS aren't auto-retried blindly — only error-class outcomes, not `reject` verdicts.)

### Frontend
- `src/ipc/runtime.ts` — `runReport(runId)` + `retryErroredInRun(runId)` + the `RunReport`/`InvocationRow`/`RunHealth` TS types.
- `src/hooks/useRunReport.ts` — fetches `run_report(selectedRun.id)`, refetches on `run-changed` / `task-changed` / `usage-changed`; returns `{ report, loading, reload }`.
- `src/components/RunHealthBanner.tsx` — verdict dot + state label + reason + contextual recovery buttons (wired to the existing per-item commands + `retryErroredInRun` + brake). Shared by RunView and BoardView.
- `src/components/RunView.tsx` — banner + rollup (started, #invocations, cost, counts) + the audit table (stage, item, outcome [tokenized color], tokens, duration, attempts, artifact link → opens the artifact via the existing `read_artifact`/CardDrawer path).
- `src/components/ViewSwitcher.tsx` — add a `run` tab (roving-tabindex already handled by FE2).
- `src/App.tsx` — route `view==="run"` → `<RunView>`; pass the health from `useRunReport` into `<BoardView>` so it renders `<RunHealthBanner>` at the top of the board.

### Data flow
`run_report(run_id)` → reads runs + audit(`list_for_run`) + stores + tasks → joins audit↔tasks, sums cost, computes counts, calls `diagnose_run` → `RunReport`. Frontend `useRunReport` fetches it (run-scoped) + refetches on the existing events. RunView and the board banner both read the same report. Recovery buttons call existing commands → emit `task-changed`/`run-changed` → the report refetches → verdict updates.

## Error handling
- No run selected → empty state ("no run scoped").
- `run_report` is best-effort: a sub-read failure yields a partial report (and a `Running`/"no data" verdict) rather than erroring the view.
- `diagnose_run` on empty/no-audit data → `Running`/"no data yet" (never panics; `now` injected).
- `retry_errored_in_run` with zero errored items → no-op, returns 0.

## Testing (no live `claude`)
- **`diagnose_run` (pure):** one test per state + precedence (e.g. error-loop beats needs-human; a full store with a recent settle is NOT backpressure); the time-window logic with injected `now` (settle 2m ago ≠ stalled; 4m ago = stalled); empty input → Running.
- **`list_for_run` + `run_report`:** seed `invocation_audit` + `tasks` + `stores` for a run → assert rows (incl. a `gen:` row with no task → empty item_key/no artifact), the cost sum (all-tokens), counts, and the embedded verdict. FK-less test pools as elsewhere.
- **`retry_errored_in_run`:** seed needs_human tasks (one error-outcome, one reject-outcome) → only the error one is re-queued (`queued`, attempts reset); reject untouched; returns 1.
- **Frontend:** `RunHealthBanner` renders the verdict + the right contextual actions per state (and clicking calls the right command); `RunView` renders the rollup + audit table (outcome colors, artifact link); `useRunReport` refetches on a `task-changed` event; ViewSwitcher shows the `run` tab.

## Out of scope / non-goals
- **Log persistence / replay** of a settled worker's output+thinking (decided out; live-only stays as-is).
- **New mutation commands beyond `retry_errored_in_run`** — all other recovery reuses existing per-item commands.
- **Cross-run dashboard / history-of-runs analytics** — the inspector is scoped to one run (the selected run); the run dropdown (history) already lists runs.
- **Notifications** (the deferred operability option B) — separate feature.
- **Schema changes** — none; this is pure read + one read method on an existing store.

## Relationship to other items
- Surfaces the **R3 `invocation_audit`** data (which shipped with "no frontend surface") and the **④e stores/occupancy** + **runs** state.
- Recovery reuses **H/L2** (`retry_task`/`force_advance`/`abandon_task`/`accept_task`) + the **lifecycle** brake.
- Cost basis matches **LF34** (all-tokens incl. cache).
- Run scoping reuses **`plan-run-lifecycle-ux`** (`selectedRun`, the one-active-run model); the board banner pairs with the existing board.
