---
id: vet-r3-invocation-audit-2026-06-23
target: plans/2026-06-23-plan-r3-invocation-audit.md
verb: vet
lens: strategic · vet · brief
date: 2026-06-23
verdict: SOUND — proceed; two minor doc-clarity amendments (F1, F2) applied
---

# Vet — R3 Per-invocation persistence / audit

Reviewed the plan against `DOMAIN.md` and the affected code (`runtime/src/pool.rs`,
`runtime/src/task_store.rs`, `runners/src/output.rs`, `usage_telemetry/src/worker_log.rs`,
`app/src/lib.rs`, `app/migrations/006_fanout.sql`). DDD soundness only.

## Summary

The design is **sound**. Ownership lands in **Runtime** (not Runners, not a new Audit
context); the start/settle writes are a clean additive seam mirroring the three existing
`PoolContext` collaborators (`usage_sink` / `revision_reader` / `log_sink`); there is **no
cross-aggregate transaction**; migration discipline is intact (007 append-only, 001–006
untouched, both migration lists synced); naming is on-language with DOMAIN.md's "Invocation".
No structural (§E) findings. Two minor doc-clarity amendments below, both applied.

## Focus questions (asked by the operator)

**Where does the record + store belong — Runtime, Runners, or a small Audit context?**
→ **Runtime.** DOMAIN.md files "Invocation" under the *Runners (ACL)* vocabulary ("one
Claude call"), but Runners is deliberately a **stateless** anti-corruption layer — it
translates idioms and touches no SQLite. The invocation *lifecycle* (start→settle) is owned
by the WorkerPool in `pool.rs::process_one_claim`. The record is *about* a Runners concept
but *written by* Runtime's loop — exactly how Runtime already writes `worker_usage` (via the
UsageSink seam) and owns `TaskStore`. Putting a store in Runners would break its
statelessness (a worse boundary violation). A standalone "Audit" bounded context was
considered and rejected: a single behavior-less append-only table with no consumer (viewer
out of scope) does not warrant its own ubiquitous language + integration relationship — that
would be a premature context. Recorded as a non-blocking note (N1).

**Two-aggregate Task/WorkerPool model — any cross-root transaction?**
→ **No.** Plan D2 is explicit: `invocation_audit` is its own append-only root keyed by a
fresh `invocation_id`; the start write and settle update touch only that row and are never
folded into the Task `claim`/`settle` UPDATE. Two separate awaited statements, best-effort
(logged, never propagated) — mirroring the existing UsageSink discipline ("a telemetry write
must never fail a settle", `worker_log.rs`). An in-flight row (`outcome IS NULL`) left by a
crash is the intended "started but never completed" signal; crash *recovery* stays
`release_orphaned_running` (D7). Clean.

**Is start/settle a clean addition?** → Yes — additive, mirrors `usage_sink`/`revision_reader`/
`log_sink` (`Option<…>`, `None` = no-op for runtime-only tests). Concrete `Arc<InvocationAuditStore>`
(not a `dyn` trait) is correct here because the store lives *inside* Runtime like `TaskStore`,
unlike UsageSink which crosses into Telemetry and therefore is a trait. No needless abstraction
(§E "adds where a refactor fits" cleared — `worker_usage_log` has a different grain and is
Telemetry's table; `TaskStore` is the Task aggregate; neither fits).

**Migration discipline 001–006 untouched?** → Yes. 007 is `CREATE TABLE IF NOT EXISTS` +
indexes only. Plan D6 registers 007 in BOTH migration registries in `app/src/lib.rs`
(`run_migrations` and the `tauri_plugin_sql` vec) and bumps the `migration_tests`
`user_version` assertion 6 → 7 — verified both lists and the test exist in the real file.

**Naming vs DOMAIN.md "Invocation"?** → On-language. `invocation_id` / `InvocationAudit` /
`InvocationAuditStore`. Crucially, the outcome encoding keeps "verdict" meaning *a settled
model judgment*: the operational synthetic-revise fallback (pool's D10 non-rate-limit branch)
is audited as the **error class**, not as `revise` — so the trail never misreports an
operational failure as a model verdict. Linguistically precise.

## Findings

### F1 [low] off-language / clarity — audit "outcome" is the *invocation* outcome, not the Task transition

**What:** On the success path, `settle_audit` records the verdict *before* `settle_and_route`.
If routing later fails, the audit row reads `approve` though the Task never advanced. A future
reader could mistake this for an inconsistency and "fix" it by moving the audit write into the
Task transaction — reintroducing the cross-root coupling D2 forbids.
**Cited plan section:** Task 3 Step 4 (success-path settle ordering); D2.
**Why it matters:** The audit records the *Claude call's* outcome (per DOMAIN.md "Invocation =
one Claude call"), which is true independent of whether the Task transition then persisted.
The ordering is correct; only its *intent* is implicit.
**Amendment:** Add a doc-comment on `settle_audit` (and a one-line note at the success-path
call site) stating the audit captures the invocation outcome, deliberately outside the Task
transaction (D2) — do not move it inside.
**Status:** resolved — amendment folded into Task 3 (doc-comment on `settle_audit` + call-site note).

### F2 [low] clarity — `attempts` column semantics

**What:** The audit `attempts` column stores the *Task's* attempt counter at invoke time, not
an invocation-local count. Without a note this reads ambiguously next to "one row per invocation".
**Cited plan section:** Task 1 SQL (`attempts` column); Task 2 `record_start`.
**Why it matters:** Prevents a future reader treating it as an invocation retry count.
**Amendment:** The Task 1 SQL comment already says "the task's attempt no. at invoke time" —
keep it, and mirror the phrasing in the `record_start` doc-comment.
**Status:** resolved — Task 1 comment already present; `record_start` doc-comment mirrors it.

## Non-blocking notes

- **N1 — no standalone Audit context (yet).** The table lives in Runtime as a supporting
  concern. If a future cross-context audit viewer reads invocations *and* gates *and* comments
  together, revisit whether an Audit read-model context earns its keep. Not now (YAGNI).

## Verdict

**SOUND — proceed.** No structural §E smells. F1/F2 are doc-clarity only and are applied to
the plan; no redesign, no separate `remediate` pass required. Migration discipline, ownership,
transaction boundary, and language all hold.
