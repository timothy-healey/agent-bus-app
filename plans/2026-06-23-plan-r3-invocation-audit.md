# R3 — Per-invocation persistence / audit — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist a durable audit record per Invocation — a `start` row when the pool begins an invocation and a `settle` update when it finishes — capturing task_id, team, model, timestamps, outcome (verdict or error class), and usage, so there is a durable trail (today Invocations are ephemeral; crash recovery is Task-claim release only).

**Architecture:** Runtime owns the worker lifecycle, so the audit record + store live in the **Runtime** context (new `runtime/src/invocation_audit.rs`), mirroring `TaskStore`'s sqlx patterns. The pool writes via a `Option<Arc<InvocationAuditStore>>` seam on `PoolContext` — exactly the no-op-when-`None` shape the existing `usage_sink` / `revision_reader` / `log_sink` collaborators use, so all existing runtime-only tests stay green with `None`. The audit table is **append-only and standalone** (its own root): the start write and the settle update are on the audit row only and are *never* in the same transaction as the Task aggregate's claim/settle — no cross-aggregate transaction. A best-effort audit write never fails a settle (same discipline as `UsageSink`). Migration `007_invocation_audit.sql` is append-only; 001–006 untouched.

**Tech Stack:** Rust (Cargo workspace under `src-tauri/`), `sqlx` (SQLite), `tokio`, `async-trait`, `thiserror`. Verify with `cargo test --workspace`, `cargo check --workspace`, `cargo clippy --workspace`, `bun vitest run`, `bun run build`.

---

## Decisions

- **D1 — Ownership: Runtime.** The Invocation lifecycle (start→settle) is the WorkerPool's job; Runtime already owns `process_one_claim`, `TaskStore`, and the `release_orphaned_running` crash-recovery seam. The audit record is *about* an Invocation (Runners' vocabulary word) but is *written by* Runtime's worker loop. Placing the store in Runtime mirrors `TaskStore` and keeps Runners a pure stateless ACL (it never touches SQLite). Confirmed by DDD vet.
- **D2 — Standalone append-only table, not part of the Task aggregate.** `invocation_audit` is its own row keyed by an `invocation_id` (a fresh uuid per invocation). The start-write and settle-update touch only that row. We deliberately do NOT join it into the `tasks` UPDATE — that would create a cross-root write coupling Task's transaction to the audit. Two separate awaited statements; if the second is lost to a crash the row simply stays `outcome = NULL` (in-flight), which is itself the audit signal the design wants ("start + completion").
- **D3 — Seam shape: `Option<Arc<InvocationAuditStore>>` on `PoolContext`.** Matches `usage_sink`/`revision_reader`/`log_sink`. `None` = no audit (runtime-only unit tests, pre-project boot). A concrete store is wired at the composition root over the app pool. We use the concrete store type (not a `dyn` trait) because — unlike `UsageSink` which bridges to *another* context (Telemetry) — the audit store lives *inside* Runtime, exactly like `TaskStore` which the pool already holds concretely. No trait indirection is warranted (YAGNI).
- **D4 — Outcome encoding.** `outcome` is a single TEXT column: `NULL` while in-flight, then one of `approve` / `revise` / `reject` (a settled verdict, lowercased to match `Verdict`'s serde) OR an error class `error:rate_limited` / `error:spawn` / `error:no_result` / `error:other` (the operational-failure path). A separate `outcome_kind` column (`verdict` | `error`) makes querying unambiguous without string-prefix parsing. The synthetic-revise operational fallback (pool's non-rate-limit error branch) is audited as the **error class**, not as `revise`, so the audit trail does not misreport an operational failure as a model verdict (consistent with the pool's D10 note).
- **D5 — Usage on settle.** The settle update records `input_tokens`/`output_tokens`/`cache_creation`/`cache_read`/`model` from `RunnerOutput.usage` on the success path; on the error path usage columns stay 0 and `model` falls back to the requested model (no usage was produced). This mirrors the existing `usage_sink` source (`output.usage`).
- **D6 — Two migration lists.** `app/src/lib.rs` has TWO migration registries that must stay in sync: `run_migrations` (the real boot path, gated on `PRAGMA user_version`) and the `tauri_plugin_sql` `migrations` vec (dormant — the frontend never calls `Database.load()`, but kept consistent). Register `007` in BOTH, and bump the `migration_tests` assertion `user_version == 6` → `== 7`.
- **D7 — No crash-recovery behaviour change.** R3 is audit-only. `release_orphaned_running` stays the correctness mechanism. An in-flight (`outcome IS NULL`) audit row left by a crash is intentionally left as-is — it is the durable "started but never completed" trail. No reconciliation logic in v1.1 (YAGNI; a viewer is out of scope).

---

## File Structure

- **Create** `src-tauri/app/migrations/007_invocation_audit.sql` — the append-only audit table + indexes.
- **Create** `src-tauri/runtime/src/invocation_audit.rs` — `InvocationAudit` record, `InvocationOutcome`, `InvocationAuditStore` (sqlx, mirrors `TaskStore`), unit tests.
- **Modify** `src-tauri/runtime/src/lib.rs` — `pub mod invocation_audit;` + re-export.
- **Modify** `src-tauri/runtime/src/pool.rs` — add `audit: Option<Arc<InvocationAuditStore>>` to `PoolContext`; write start after claim, settle after outcome known; update test `ctx_with` + `fresh_pool` to include migration 007 and `audit: None`; add an audit-path test.
- **Modify** `src-tauri/app/src/lib.rs` — register migration 007 in both lists; bump migration test; construct + wire the `InvocationAuditStore` into every `PoolContext` via `spawn_worker_loops`.

---

### Task 1: Migration `007_invocation_audit.sql`

**Files:**
- Create: `src-tauri/app/migrations/007_invocation_audit.sql`

- [ ] **Step 1: Write the migration**

```sql
-- 007_invocation_audit.sql — R3 (per-invocation persistence / audit). Adds a
-- durable, append-only audit trail: one row per Invocation (DOMAIN.md → Runners
-- "Invocation": one Claude call). Written by Runtime's WorkerPool at invoke-start
-- (outcome NULL = in-flight) and updated at settle with the verdict/error class +
-- usage. Standalone root — never joined into the tasks UPDATE; no cross-aggregate
-- transaction. Migrations 001–006 are never edited.
--
-- An in-flight row (outcome IS NULL) left by a Runtime crash is the intended
-- "started but never completed" signal; crash *recovery* stays Task-claim release
-- (release_orphaned_running). v1.1 adds no reconciliation (a viewer is out of scope).
CREATE TABLE IF NOT EXISTS invocation_audit (
  invocation_id   TEXT PRIMARY KEY,             -- fresh uuid per invocation
  task_id         TEXT NOT NULL,
  team_id         TEXT NOT NULL,
  model           TEXT NOT NULL,
  attempts        INTEGER NOT NULL DEFAULT 0,    -- the task's attempt no. at invoke time
  started_at      INTEGER NOT NULL,
  settled_at      INTEGER,                       -- NULL while in-flight
  outcome_kind    TEXT,                          -- NULL | 'verdict' | 'error'
  outcome         TEXT,                          -- NULL | 'approve'|'revise'|'reject' | 'error:<class>'
  input_tokens    INTEGER NOT NULL DEFAULT 0,
  output_tokens   INTEGER NOT NULL DEFAULT 0,
  cache_creation  INTEGER NOT NULL DEFAULT 0,
  cache_read      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_invocation_audit_task ON invocation_audit(task_id);
CREATE INDEX IF NOT EXISTS idx_invocation_audit_started ON invocation_audit(started_at);
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/app/migrations/007_invocation_audit.sql
git commit -m "feat(r3): add 007_invocation_audit migration (append-only audit table)"
```

---

### Task 2: `InvocationAudit` record + `InvocationAuditStore`

**Files:**
- Create: `src-tauri/runtime/src/invocation_audit.rs`
- Modify: `src-tauri/runtime/src/lib.rs`

- [ ] **Step 1: Write the module with failing tests**

Create `src-tauri/runtime/src/invocation_audit.rs`:

```rust
//! InvocationAuditStore — SQLite persistence for the per-invocation audit trail
//! (R3). One row per Invocation (DOMAIN.md → Runners "Invocation": one Claude
//! call). The WorkerPool writes a `start` row right after it claims a task (before
//! invoking the runner) and an `settle` update once the outcome is known. The
//! table is a standalone append-only root: these writes touch only the audit row
//! and are never in the same transaction as the Task aggregate's claim/settle.
//! Mirrors TaskStore's sqlx patterns.

use agent_bus_core::Verdict;
use runners::output::RunnerError;
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InvocationAuditError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// The terminal outcome of one invocation: a settled model verdict, or an
/// operational error class (the invocation broke and produced no verdict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvocationOutcome {
    Verdict(Verdict),
    Error(ErrorClass),
}

/// Coarse error classification for the audit trail. Mirrors RunnerError's variants
/// (without the messages) so a persistently-failing team is visible in the trail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    RateLimited,
    Spawn,
    NoResult,
    Other,
}

impl ErrorClass {
    /// Classify a RunnerError into a coarse audit class.
    pub fn of(e: &RunnerError) -> Self {
        match e {
            RunnerError::RateLimited(_) => ErrorClass::RateLimited,
            RunnerError::Spawn(_) => ErrorClass::Spawn,
            RunnerError::NoResult => ErrorClass::NoResult,
            RunnerError::Other(_) => ErrorClass::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorClass::RateLimited => "rate_limited",
            ErrorClass::Spawn => "spawn",
            ErrorClass::NoResult => "no_result",
            ErrorClass::Other => "other",
        }
    }
}

impl InvocationOutcome {
    /// (outcome_kind, outcome) column values for this outcome.
    fn columns(&self) -> (&'static str, String) {
        match self {
            InvocationOutcome::Verdict(v) => {
                let s = match v {
                    Verdict::Approve => "approve",
                    Verdict::Revise => "revise",
                    Verdict::Reject => "reject",
                };
                ("verdict", s.to_string())
            }
            InvocationOutcome::Error(c) => ("error", format!("error:{}", c.as_str())),
        }
    }
}

/// What the settle write records about token usage. Sourced from RunnerOutput.usage
/// on success; zeroed on the error path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditUsage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

/// One audit row as read back (for tests / a future viewer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationAudit {
    pub invocation_id: String,
    pub task_id: String,
    pub team_id: String,
    pub model: String,
    pub attempts: u32,
    pub started_at: i64,
    pub settled_at: Option<i64>,
    pub outcome_kind: Option<String>,
    pub outcome: Option<String>,
    pub usage: AuditUsage,
}

pub struct InvocationAuditStore {
    pool: SqlitePool,
}

impl InvocationAuditStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Write the start row (outcome NULL = in-flight). Returns the row's
    /// invocation_id (a fresh uuid) so the caller can settle it later. `attempts`
    /// is the *Task's* attempt counter at invoke time, not an invocation-local
    /// count (VET F2).
    pub async fn record_start(
        &self,
        task_id: &str,
        team_id: &str,
        model: &str,
        attempts: u32,
        started_at: i64,
    ) -> Result<String, InvocationAuditError> {
        let invocation_id = format!("I-{}", uuid::Uuid::new_v4());
        sqlx::query(
            "INSERT INTO invocation_audit
               (invocation_id, task_id, team_id, model, attempts, started_at)
             VALUES (?,?,?,?,?,?)",
        )
        .bind(&invocation_id)
        .bind(task_id)
        .bind(team_id)
        .bind(model)
        .bind(attempts as i64)
        .bind(started_at)
        .execute(&self.pool)
        .await?;
        Ok(invocation_id)
    }

    /// Update the row with its terminal outcome + usage. Idempotent on the PK.
    pub async fn record_settle(
        &self,
        invocation_id: &str,
        outcome: &InvocationOutcome,
        usage: &AuditUsage,
        settled_at: i64,
    ) -> Result<(), InvocationAuditError> {
        let (kind, outcome_str) = outcome.columns();
        sqlx::query(
            "UPDATE invocation_audit SET
               settled_at=?, outcome_kind=?, outcome=?, model=?,
               input_tokens=?, output_tokens=?, cache_creation=?, cache_read=?
             WHERE invocation_id=?",
        )
        .bind(settled_at)
        .bind(kind)
        .bind(&outcome_str)
        .bind(&usage.model)
        .bind(usage.input_tokens as i64)
        .bind(usage.output_tokens as i64)
        .bind(usage.cache_creation as i64)
        .bind(usage.cache_read as i64)
        .bind(invocation_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Read one row back (tests / future viewer).
    pub async fn get(&self, invocation_id: &str) -> Result<Option<InvocationAudit>, InvocationAuditError> {
        type R = (
            String, String, String, String, i64, i64,
            Option<i64>, Option<String>, Option<String>, i64, i64, i64, i64,
        );
        let row = sqlx::query_as::<_, R>(
            "SELECT invocation_id, task_id, team_id, model, attempts, started_at,
                    settled_at, outcome_kind, outcome,
                    input_tokens, output_tokens, cache_creation, cache_read
             FROM invocation_audit WHERE invocation_id = ?",
        )
        .bind(invocation_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| InvocationAudit {
            invocation_id: r.0,
            task_id: r.1,
            team_id: r.2,
            model: r.3.clone(),
            attempts: r.4 as u32,
            started_at: r.5,
            settled_at: r.6,
            outcome_kind: r.7,
            outcome: r.8,
            usage: AuditUsage {
                model: r.3,
                input_tokens: r.9 as u64,
                output_tokens: r.10 as u64,
                cache_creation: r.11 as u64,
                cache_read: r.12 as u64,
            },
        }))
    }

    /// All rows for a task, oldest first (tests / future viewer).
    pub async fn list_for_task(&self, task_id: &str) -> Result<Vec<InvocationAudit>, InvocationAuditError> {
        type R = (
            String, String, String, String, i64, i64,
            Option<i64>, Option<String>, Option<String>, i64, i64, i64, i64,
        );
        let rows = sqlx::query_as::<_, R>(
            "SELECT invocation_id, task_id, team_id, model, attempts, started_at,
                    settled_at, outcome_kind, outcome,
                    input_tokens, output_tokens, cache_creation, cache_read
             FROM invocation_audit WHERE task_id = ? ORDER BY started_at ASC",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| InvocationAudit {
                invocation_id: r.0,
                task_id: r.1,
                team_id: r.2,
                model: r.3.clone(),
                attempts: r.4 as u32,
                started_at: r.5,
                settled_at: r.6,
                outcome_kind: r.7,
                outcome: r.8,
                usage: AuditUsage {
                    model: r.3,
                    input_tokens: r.9 as u64,
                    output_tokens: r.10 as u64,
                    cache_creation: r.11 as u64,
                    cache_read: r.12 as u64,
                },
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/007_invocation_audit.sql"))
            .execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn start_then_settle_records_verdict_and_usage() {
        let store = InvocationAuditStore::new(fresh_pool().await);
        let id = store.record_start("T-1", "research", "claude-opus-4-8", 1, 1000).await.unwrap();

        // in-flight: outcome NULL
        let row = store.get(&id).await.unwrap().unwrap();
        assert_eq!(row.task_id, "T-1");
        assert_eq!(row.team_id, "research");
        assert_eq!(row.attempts, 1);
        assert_eq!(row.started_at, 1000);
        assert_eq!(row.settled_at, None);
        assert_eq!(row.outcome, None);
        assert_eq!(row.outcome_kind, None);

        let usage = AuditUsage { model: "claude-opus-4-8".into(), input_tokens: 100, output_tokens: 20, cache_creation: 5, cache_read: 3 };
        store.record_settle(&id, &InvocationOutcome::Verdict(Verdict::Approve), &usage, 1100).await.unwrap();

        let row = store.get(&id).await.unwrap().unwrap();
        assert_eq!(row.settled_at, Some(1100));
        assert_eq!(row.outcome_kind.as_deref(), Some("verdict"));
        assert_eq!(row.outcome.as_deref(), Some("approve"));
        assert_eq!(row.usage.input_tokens, 100);
        assert_eq!(row.usage.output_tokens, 20);
        assert_eq!(row.usage.cache_creation, 5);
        assert_eq!(row.usage.cache_read, 3);
    }

    #[tokio::test]
    async fn settle_records_error_class() {
        let store = InvocationAuditStore::new(fresh_pool().await);
        let id = store.record_start("T-2", "writers", "m", 2, 1).await.unwrap();
        let err = RunnerError::RateLimited("429".into());
        store
            .record_settle(&id, &InvocationOutcome::Error(ErrorClass::of(&err)), &AuditUsage::default(), 2)
            .await
            .unwrap();
        let row = store.get(&id).await.unwrap().unwrap();
        assert_eq!(row.outcome_kind.as_deref(), Some("error"));
        assert_eq!(row.outcome.as_deref(), Some("error:rate_limited"));
    }

    #[tokio::test]
    async fn list_for_task_orders_by_started_at() {
        let store = InvocationAuditStore::new(fresh_pool().await);
        store.record_start("T-3", "research", "m", 1, 10).await.unwrap();
        store.record_start("T-3", "research", "m", 2, 20).await.unwrap();
        store.record_start("T-other", "research", "m", 1, 15).await.unwrap();
        let rows = store.list_for_task("T-3").await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].started_at, 10);
        assert_eq!(rows[1].started_at, 20);
    }

    #[test]
    fn error_class_maps_every_runner_error_variant() {
        assert_eq!(ErrorClass::of(&RunnerError::RateLimited("x".into())).as_str(), "rate_limited");
        assert_eq!(ErrorClass::of(&RunnerError::Spawn("x".into())).as_str(), "spawn");
        assert_eq!(ErrorClass::of(&RunnerError::NoResult).as_str(), "no_result");
        assert_eq!(ErrorClass::of(&RunnerError::Other("x".into())).as_str(), "other");
    }
}
```

- [ ] **Step 2: Register the module in `src-tauri/runtime/src/lib.rs`**

After the `pub mod fanout_store;` / `pub use fanout_store::*;` block (around line 31), add:

```rust
pub mod invocation_audit;
pub use invocation_audit::*;
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime invocation_audit`
Expected: PASS — `start_then_settle_records_verdict_and_usage`, `settle_records_error_class`, `list_for_task_orders_by_started_at`, `error_class_maps_every_runner_error_variant` all green.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/runtime/src/invocation_audit.rs src-tauri/runtime/src/lib.rs
git commit -m "feat(r3): InvocationAudit record + InvocationAuditStore (Runtime)"
```

---

### Task 3: Pool writes start + settle audit records

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs`

- [ ] **Step 1: Add the `audit` collaborator to `PoolContext`**

In `src-tauri/runtime/src/pool.rs`, add the import near the other crate-local `use` lines (after the `task_store` use, ~line 12):

```rust
use crate::invocation_audit::{AuditUsage, ErrorClass, InvocationAuditStore, InvocationOutcome};
```

Add the field to `PoolContext` (after the `log_sink` field, ~line 80):

```rust
    /// Per-invocation audit store (R3). None = no audit (runtime-only tests /
    /// pre-project boot). Standalone append-only root — written by start/settle
    /// below; never joined into the Task transaction.
    pub audit: Option<Arc<InvocationAuditStore>>,
```

- [ ] **Step 2: Write the start row after claim, capture the invocation_id**

In `process_one_claim`, immediately AFTER the claim block (after the `let Some(mut task) = ... else { return Ok(ClaimOutcome::Idle); };` and before `// 2. PREPARE scope`), insert:

```rust
    // R3: open an audit record for this invocation (outcome NULL = in-flight).
    // Best-effort: an audit write must never fail a settle (mirrors UsageSink).
    let effective_for_audit = team.effective_runner();
    let audit_id = match &ctx.audit {
        Some(store) => store
            .record_start(&task.id.0, &team.id, &effective_for_audit.model, task.attempts, now)
            .await
            .map_err(|e| eprintln!("runtime: invocation audit start failed: {e}"))
            .ok(),
        None => None,
    };
```

- [ ] **Step 3: Settle the audit row on the error paths**

In the `let output = match result { ... }` block, the rate-limit arm (currently returns `ClaimOutcome::RateLimited`) must settle the audit first. Replace the rate-limit arm body so it reads:

```rust
        Err(e) if e.is_rate_limited() => {
            settle_audit(ctx, &audit_id, &InvocationOutcome::Error(ErrorClass::of(&e)), &AuditUsage::default()).await;
            // Release the task back to queued (spec: rate-limit releases the
            // held task). Don't bump attempts.
            task.transition_to(TaskState::Queued, now_unix())?;
            ctx.tasks.update(&task).await?;
            return Ok(ClaimOutcome::RateLimited { task_id: task.id.0 });
        }
```

And the non-rate-limit error arm: settle the audit with the error class BEFORE the synthetic revise. Replace the `Err(_e) => { ... }` arm's first line so the arm reads:

```rust
        Err(e) => {
            settle_audit(ctx, &audit_id, &InvocationOutcome::Error(ErrorClass::of(&e)), &AuditUsage::default()).await;
            // Non-rate-limit failure (spawn/parse/NoResult): this is an
            // OPERATIONAL failure — the invocation broke and produced NO
            // verdict. We reuse the *revise* route (send back, bump attempts)
            // as a safety valve so the task isn't stranded `running`, and so a
            // persistently-failing team eventually escalates to needs_human via
            // the attempts cap rather than needing a new error state. NOTE: this
            // synthetic revise is NOT a model-produced `Verdict::Revise` (canon:
            // a settled judgment); it is an operational fallback reusing the
            // route. Plan 5 telemetry/diagnostics should not read it as a real
            // model verdict. (See Decision D10.) The AUDIT records the error
            // class, not `revise` (R3 D4), so the trail reflects the failure.
            let _ = settle_and_route(ctx, &mut task, agent_bus_core::Verdict::Revise).await?;
            let reloaded = ctx.tasks.get(&task.id).await?;
            return Ok(ClaimOutcome::Settled {
                task_id: reloaded.id.0,
                next_stage: reloaded.current_stage,
                next_state: reloaded.state,
            });
        }
```

(Note: the arm binds `e` instead of `_e` now — used by `ErrorClass::of`.)

- [ ] **Step 4: Settle the audit row on the success path**

After the usage-sink block (after the `if let Some(sink) = &ctx.usage_sink { ... }` block, before `// 6. ROUTE per verdict.`), insert:

```rust
    // R3: settle the audit record with the model's verdict + usage. This is the
    // invocation outcome, recorded outside the Task transaction (VET F1 / D2).
    settle_audit(
        ctx,
        &audit_id,
        &InvocationOutcome::Verdict(output.verdict),
        &AuditUsage {
            model: output.usage.model.clone(),
            input_tokens: output.usage.input_tokens,
            output_tokens: output.usage.output_tokens,
            cache_creation: output.usage.cache_creation,
            cache_read: output.usage.cache_read,
        },
    )
    .await;
```

- [ ] **Step 5: Add the `settle_audit` helper**

After the `process_one_claim` function (before `settle_and_route`), add:

```rust
/// Best-effort settle of an audit row (R3). No-op when audit is unwired or the
/// start write was lost; a failure is logged, never propagated — an audit write
/// must never fail a settle (mirrors UsageSink discipline).
///
/// VET F1: this records the *invocation* outcome (the Claude call's verdict/error
/// — DOMAIN.md "Invocation = one Claude call"), which is true independent of
/// whether the subsequent Task transition (`settle_and_route`) persists. It is
/// DELIBERATELY outside the Task transaction (D2) — do NOT move it inside, that
/// would reintroduce the cross-root coupling the two-aggregate model forbids.
async fn settle_audit(
    ctx: &PoolContext,
    audit_id: &Option<String>,
    outcome: &InvocationOutcome,
    usage: &AuditUsage,
) {
    if let (Some(store), Some(id)) = (&ctx.audit, audit_id) {
        if let Err(e) = store.record_settle(id, outcome, usage, now_unix()).await {
            eprintln!("runtime: invocation audit settle failed: {e}");
        }
    }
}
```

- [ ] **Step 6: Update the test harness in `pool.rs`**

In the `tests` module, add migration 007 to `fresh_pool` (after the `006_fanout.sql` line):

```rust
        sqlx::query(include_str!("../../app/migrations/007_invocation_audit.sql")).execute(&pool).await.unwrap();
```

In `ctx_with`, add `audit: None,` to the `PoolContext { ... }` literal (after `log_sink: None,`):

```rust
            log_sink: None,
            audit: None,
```

- [ ] **Step 7: Add an audit-path test**

In the `tests` module of `pool.rs`, add (after `streaming_forwards_log_deltas_per_task_and_settles_unchanged`):

```rust
    #[tokio::test]
    async fn settle_writes_an_audit_record_with_verdict_and_usage() {
        use crate::invocation_audit::InvocationAuditStore;
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("gate-1"), None)],
            vec![Gate { id: "gate-1".into(), label: "G".into(), downstream: "research".into() }]);
        let out = RunnerOutput {
            verdict: Verdict::Approve,
            artifact_path: Some("artifacts/analyses/a.md".into()),
            final_text: "VERDICT: approve".into(),
            usage: RunnerUsage { model: "claude-opus-4-7".into(), input_tokens: 100, output_tokens: 20, cache_creation: 5, cache_read: 3 },
        };
        let mut ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(out)), temp_root());
        let audit = Arc::new(InvocationAuditStore::new(pool.clone()));
        ctx.audit = Some(audit.clone());
        let t = Task::injected("proj".into(), "p".into(), "research".into(), "topic".into(), None, 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap();

        let rows = audit.list_for_task(&t.id.0).await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.team_id, "research");
        assert_eq!(row.outcome_kind.as_deref(), Some("verdict"));
        assert_eq!(row.outcome.as_deref(), Some("approve"));
        assert!(row.settled_at.is_some());
        assert_eq!(row.usage.input_tokens, 100);
        assert_eq!(row.usage.output_tokens, 20);
    }

    #[tokio::test]
    async fn rate_limit_audits_error_class_and_leaves_no_verdict() {
        use crate::invocation_audit::InvocationAuditStore;
        let pool = fresh_pool().await;
        let p = pipeline_with(vec![team("research", Some("done"), None)], vec![]);
        let runner = Arc::new(FakeRunner::new(vec![Err(RunnerError::RateLimited("429".into()))]));
        let mut ctx = ctx_with(pool.clone(), p.clone(), runner, temp_root());
        let audit = Arc::new(InvocationAuditStore::new(pool.clone()));
        ctx.audit = Some(audit.clone());
        let t = Task::injected("proj".into(), "p".into(), "research".into(), "topic".into(), None, 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap();

        let rows = audit.list_for_task(&t.id.0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome_kind.as_deref(), Some("error"));
        assert_eq!(rows[0].outcome.as_deref(), Some("error:rate_limited"));
    }
```

- [ ] **Step 8: Run the runtime tests**

Run: `cd src-tauri && cargo test -p runtime`
Expected: PASS — all existing pool/task_store tests stay green; the two new audit-path tests pass.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "feat(r3): pool writes start + settle invocation-audit records"
```

---

### Task 4: Register migration 007 + wire the store at the composition root

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Register migration 007 in `run_migrations`**

In `src-tauri/app/src/lib.rs`, in the `MIGRATIONS` const array inside `run_migrations`, add after the `(6, ...)` line:

```rust
        (7, include_str!("../migrations/007_invocation_audit.sql")),
```

- [ ] **Step 2: Register migration 007 in the `tauri_plugin_sql` list**

In `run()`, in the `migrations` vec, add after the version-6 `Migration { ... }` entry:

```rust
        Migration {
            version: 7,
            description: "invocation audit — per-invocation persistence/audit trail",
            sql: include_str!("../migrations/007_invocation_audit.sql"),
            kind: MigrationKind::Up,
        },
```

- [ ] **Step 3: Bump the migration test assertion**

In `mod migration_tests`, change the `user_version` assertion from 6 to 7:

```rust
        assert_eq!(version, 7, "all seven migrations recorded");
```

- [ ] **Step 4: Construct the audit store + wire it into the worker loops**

In `run()`'s `.setup()`, after the `TaskStore` is created (`let tasks = Arc::new(TaskStore::new(pool.clone()));`), add:

```rust
                let invocation_audit = Arc::new(runtime::invocation_audit::InvocationAuditStore::new(pool.clone()));
```

Change the `spawn_worker_loops` signature to take the audit store. Update the function definition signature (after the `log_sink` param):

```rust
    log_sink: Option<Arc<runtime::pool::LogSinkFactory>>,
    audit: Option<Arc<runtime::invocation_audit::InvocationAuditStore>>,
) {
```

Add `audit: audit.clone(),` to the `PoolContext { ... }` literal inside `spawn_worker_loops` (after `log_sink: log_sink.clone(),`).

Update the call site in `.setup()` (the `spawn_worker_loops(...)` call) to pass the audit store as the final argument:

```rust
                    spawn_worker_loops(handle.clone(), pipe.clone(), tasks.clone(), brake.clone(), project_root, Some(usage_sink.clone()), revision_reader, pool.clone(), Some(make_task_log_sink(handle.clone())), Some(invocation_audit.clone()));
```

- [ ] **Step 5: Run the app crate tests**

Run: `cd src-tauri && cargo test -p app`
Expected: PASS — `migration_tests::fresh_file_is_created_migrated_and_idempotent` passes with `user_version == 7`; all other app tests green.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(r3): register migration 007 + wire InvocationAuditStore into worker loops"
```

---

### Task 5: Full-workspace verification

- [ ] **Step 1: cargo test**

Run: `cd src-tauri && cargo test --workspace`
Expected: PASS — entire workspace green (no regressions).

- [ ] **Step 2: cargo check + clippy**

Run: `cd src-tauri && cargo check --workspace && cargo clippy --workspace -- -D warnings`
Expected: clean — no errors, no warnings.

- [ ] **Step 3: frontend (unchanged, but must stay green)**

Run (from repo root, export PATH for bun if needed): `bun vitest run && bun run build`
Expected: PASS — R3 is backend-only; no frontend surface changes.

- [ ] **Step 4: Commit (only if any incidental fix was needed)**

No code change expected here; this task is verification.

---

## Self-Review

**1. Spec coverage.** The item asks for an audit record per invocation — start + completion — with task_id, team, model, timestamps, outcome (verdict or error class), and usage. Covered: `record_start` writes task_id/team_id/model/attempts/started_at (start); `record_settle` writes settled_at/outcome_kind/outcome/usage (completion). Outcome encodes both verdict and error class (D4). Backend-only, no UI/viewer (out of scope, honored). Migration append-only, 001–006 untouched (Task 1). Two migration lists synced + test bumped (Task 4, D6). Crash recovery unchanged (D7).

**2. Placeholder scan.** No TBD/TODO/"handle errors" placeholders; every code step shows complete code.

**3. Type consistency.** `InvocationAuditStore::record_start/record_settle/get/list_for_task`, `InvocationOutcome::{Verdict,Error}`, `ErrorClass::{of,as_str}`, `AuditUsage`, and the `PoolContext.audit` field name are used identically across Tasks 2–4. `settle_audit` helper signature matches its call sites. `ErrorClass::of` covers all four `RunnerError` variants (verified against `runners/src/output.rs`).
