# R — Runtime hardening: engine observability seams + target_repo precedence

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Closes the ④d gap: the bounded-buffer engine calls `Runner::invoke` but does NOT thread usage / live-log / per-invocation-audit, and binds `${target_repo}` from the project default only. This wires those existing seams into the engine so runs are observable + correct. Backend chunk.

**Goal:** Engine worker invocations publish usage (UsageSink), stream live-log deltas (LogSinkFactory → `task-log`), record the invocation audit (R3), and apply the `${target_repo}` task>project precedence.

**Architecture:** Add the three optional collaborators to `EngineContext`; use them in `engine::invoke` (the single invoke site at `engine.rs:1161`) mirroring how the (now-deleted) single-task pool wired R3/R4/R5 (see git history pre-`cleanup-post-cutover` for the exact idioms). The `WorkerDeps` bundle already holds `usage_sink`/`log_sink`/`audit` (currently `#[allow(dead_code)]`); the activator passes them into each run's `EngineContext`.

---

## Tasks (TDD; FakeRunner + in-memory pool)

- [ ] **Task 1 — EngineContext collaborators.** Add to `EngineContext` (`runtime/src/engine.rs`): `usage_sink: Option<Arc<dyn agent_bus_core::UsageSink>>`, `log_sink: Option<Arc<crate::log_sink::LogSinkFactory>>`, `audit: Option<Arc<crate::invocation_audit::InvocationAuditStore>>`. Update all `EngineContext { .. }` constructions (engine tests + the activator) to pass `None`/the real deps. Compiles + existing tests green. Commit.
- [ ] **Task 2 — target_repo precedence.** In `engine::invoke` (`~engine.rs:1168`), build `PathVars` `${target_repo}` from `effective_target_repo(task.target_repo.as_deref(), ctx.target_repo.as_deref())` (the precedence fn already lives in engine.rs) instead of `ctx.target_repo` alone — so a work-item's own `target_repo` overrides the project default. (Work-items carry none in v1, so behavior is unchanged today, but the rule is now applied.) Test: a task with a `target_repo` overrides the project default in the built PathVars/scope. Commit.
- [ ] **Task 3 — Invocation audit (R3) in the engine.** In `engine::invoke`: `record_start` (task_id, team, model, attempts, now) before the runner call when `ctx.audit` is set; after, `record_settle` with the outcome — `Verdict(...)` on a parsed item-list, or the error class (`rate_limited`/`spawn`/`no_result`/`model_unavailable`/`other`) on failure — plus usage. Best-effort (an audit write never fails a settle). Test with a `FakeRunner` + an in-memory `InvocationAuditStore`: a start row then a settled row with the right outcome; a failing invoke records the error class. Commit.
- [ ] **Task 4 — Usage (R5/Telemetry) in the engine.** After a successful invoke, publish a `UsageEvent` (ts, team_id, task_id, model, tokens) via `ctx.usage_sink` when set (best-effort). Test: a fake `UsageSink` receives the event with the parsed usage. Commit.
- [ ] **Task 5 — Live-log streaming (R4) in the engine.** When `ctx.log_sink` is set, call `ctx.runner.invoke_stream(&req, &sink)` (sink = `factory(task_id)`) instead of `invoke`, so prose deltas stream (the composition root coalesces them into `task-log` events). Identical parse/settle on the returned output. Test: a `FakeRunner::with_deltas` + a capturing sink receives the deltas; the settle is unchanged. Commit.
- [ ] **Task 6 — Wire at the composition root.** In `pipeline_activator.rs`: pass `WorkerDeps`'s `usage_sink`/`log_sink`/`audit` into each `EngineContext` it builds; drop the now-unneeded `#[allow(dead_code)]`. Confirm boot still builds them (`make_task_log_sink`, the usage sink, the audit store). The loop already emits `task-changed`/`usage-changed`; live-log now actually flows. Commit.

## Verification gates
- `cd src-tauri && cargo test --workspace`
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `npx tsc --noEmit`; `bun run build`
- Tag: `git tag plan-R-runtime-hardening`

## Spec coverage
- usage/live-log/audit threaded into the engine invoke (④d gap) → Tasks 1,3,4,5,6. ✓
- task>project `target_repo` precedence applied → Task 2. ✓
- (Proving the live `claude` path end-to-end is a follow-up LIVE TEST the operator drives — not in this build.)

## Constraints
Local commits on `main`, NEVER push. Commit per task. All three side-channels are best-effort (never fail a settle), mirroring the old pool. The Runner trait stays unchanged. Live `claude` paths remain structural-only (FakeRunner). Reference the pre-`cleanup-post-cutover` `pool.rs` in git history for the exact R3/R4/R5 idioms.
