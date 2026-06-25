//! InvocationAuditStore — SQLite persistence for the per-invocation audit trail
//! (R3). One row per Invocation (DOMAIN.md → Runners "Invocation": one Claude
//! call). The WorkerPool writes a `start` row right after it claims a task (before
//! invoking the runner) and a `settle` update once the outcome is known. The
//! table is a standalone append-only root: these writes touch only the audit row
//! and are never in the same transaction as the Task aggregate's claim/settle.
//! Mirrors TaskStore's sqlx patterns.

use agent_bus_core::Verdict;
use runners::output::RunnerError;
use serde::{Deserialize, Serialize};
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
    /// The model was not found / unavailable (G6) — distinct so a misconfigured
    /// model is visible in the trail (dovetails with L3 card-side detail).
    ModelUnavailable,
    Spawn,
    NoResult,
    Other,
}

impl ErrorClass {
    /// Classify a RunnerError into a coarse audit class.
    pub fn of(e: &RunnerError) -> Self {
        match e {
            RunnerError::RateLimited(_) => ErrorClass::RateLimited,
            RunnerError::ModelUnavailable(_) => ErrorClass::ModelUnavailable,
            RunnerError::Spawn(_) => ErrorClass::Spawn,
            RunnerError::NoResult => ErrorClass::NoResult,
            RunnerError::Other(_) => ErrorClass::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorClass::RateLimited => "rate_limited",
            ErrorClass::ModelUnavailable => "model_unavailable",
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

/// The L3 read DTO — one settled (or in-flight) invocation as the CardDrawer
/// history panel surfaces it. This is the ONLY shape that crosses the Runtime OHS
/// for `list_invocations`: the audit idiom (the split `outcome_kind` / `outcome`
/// columns, the raw usage struct) stays sealed behind it. `outcome` is the single
/// encoded string the frontend classifier reads: `verdict:approve|revise|reject`
/// for a settled model verdict, `error:<class>` for an operational failure, or
/// empty for an in-flight (not-yet-settled) row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationRow {
    pub invocation_id: String,
    pub team_id: String,
    pub model: String,
    pub attempts: u32,
    pub started_at: i64,
    pub settled_at: Option<i64>,
    /// `verdict:<x>` / `error:<class>` / `""` (in-flight). See the struct doc.
    pub outcome: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl From<InvocationAudit> for InvocationRow {
    fn from(a: InvocationAudit) -> Self {
        // Compose the single encoded `outcome` string from the split audit
        // columns: a verdict row carries `outcome_kind = "verdict"` + a bare
        // `approve|revise|reject`; an error row already stores `error:<class>`.
        // An unsettled row (both NULL) encodes as the empty string.
        let outcome = match (a.outcome_kind.as_deref(), a.outcome.as_deref()) {
            (Some("verdict"), Some(v)) => format!("verdict:{v}"),
            (_, Some(o)) => o.to_string(),
            _ => String::new(),
        };
        InvocationRow {
            invocation_id: a.invocation_id,
            team_id: a.team_id,
            model: a.model,
            attempts: a.attempts,
            started_at: a.started_at,
            settled_at: a.settled_at,
            outcome,
            input_tokens: a.usage.input_tokens,
            output_tokens: a.usage.output_tokens,
        }
    }
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

type AuditRow = (
    String,
    String,
    String,
    String,
    i64,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
    i64,
    i64,
    i64,
    i64,
);

fn row_to_audit(r: AuditRow) -> InvocationAudit {
    InvocationAudit {
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
    }
}

pub struct InvocationAuditStore {
    pool: SqlitePool,
}

impl InvocationAuditStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    const SELECT: &'static str =
        "SELECT invocation_id, task_id, team_id, model, attempts, started_at,
                settled_at, outcome_kind, outcome,
                input_tokens, output_tokens, cache_creation, cache_read
         FROM invocation_audit";

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
        let row = sqlx::query_as::<_, AuditRow>(&format!("{} WHERE invocation_id = ?", Self::SELECT))
            .bind(invocation_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(row_to_audit))
    }

    /// All rows for a task, oldest first (tests / future viewer).
    pub async fn list_for_task(&self, task_id: &str) -> Result<Vec<InvocationAudit>, InvocationAuditError> {
        let rows = sqlx::query_as::<_, AuditRow>(&format!(
            "{} WHERE task_id = ? ORDER BY started_at ASC",
            Self::SELECT
        ))
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_audit).collect())
    }

    /// The L3 read: every invocation for a task as the sealed `InvocationRow`
    /// DTO, NEWEST FIRST (the card's headline reason is `rows[0]`). The audit
    /// idiom does not leak — only `InvocationRow` crosses the OHS.
    pub async fn list_rows_for_task(
        &self,
        task_id: &str,
    ) -> Result<Vec<InvocationRow>, InvocationAuditError> {
        let rows = sqlx::query_as::<_, AuditRow>(&format!(
            "{} WHERE task_id = ? ORDER BY started_at DESC",
            Self::SELECT
        ))
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_audit).map(InvocationRow::from).collect())
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

    #[tokio::test]
    async fn list_rows_for_task_is_newest_first_and_encodes_outcome() {
        let store = InvocationAuditStore::new(fresh_pool().await);
        // oldest: a settled approve verdict with usage.
        let i1 = store.record_start("T-row", "research", "m1", 1, 10).await.unwrap();
        let usage = AuditUsage { model: "m1".into(), input_tokens: 7, output_tokens: 3, cache_creation: 0, cache_read: 0 };
        store.record_settle(&i1, &InvocationOutcome::Verdict(Verdict::Approve), &usage, 11).await.unwrap();
        // newest: a settled error (the headline reason).
        let i2 = store.record_start("T-row", "writers", "m2", 2, 20).await.unwrap();
        let err = RunnerError::ModelUnavailable("nope".into());
        store.record_settle(&i2, &InvocationOutcome::Error(ErrorClass::of(&err)), &AuditUsage::default(), 21).await.unwrap();

        let rows = store.list_rows_for_task("T-row").await.unwrap();
        assert_eq!(rows.len(), 2);
        // newest-first: the error row leads (the card's headline).
        assert_eq!(rows[0].team_id, "writers");
        assert_eq!(rows[0].outcome, "error:model_unavailable");
        assert_eq!(rows[0].attempts, 2);
        // the verdict row encodes with the `verdict:` prefix + carries usage.
        assert_eq!(rows[1].team_id, "research");
        assert_eq!(rows[1].outcome, "verdict:approve");
        assert_eq!(rows[1].input_tokens, 7);
        assert_eq!(rows[1].output_tokens, 3);
    }

    #[tokio::test]
    async fn list_rows_in_flight_row_encodes_empty_outcome() {
        let store = InvocationAuditStore::new(fresh_pool().await);
        store.record_start("T-if", "research", "m", 1, 10).await.unwrap();
        let rows = store.list_rows_for_task("T-if").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, "", "an unsettled row has no outcome yet");
        assert_eq!(rows[0].settled_at, None);
    }

    #[test]
    fn invocation_row_wire_contract_serializes_camel_free_snake_fields() {
        // Wire-contract: the JSON the OHS emits carries exactly these snake_case
        // fields (the TS `InvocationRow` mirrors them). Seals the DTO shape.
        let row = InvocationRow {
            invocation_id: "I-1".into(),
            team_id: "research".into(),
            model: "claude-opus-4-8".into(),
            attempts: 2,
            started_at: 100,
            settled_at: Some(110),
            outcome: "verdict:reject".into(),
            input_tokens: 50,
            output_tokens: 12,
        };
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(v["invocation_id"], "I-1");
        assert_eq!(v["team_id"], "research");
        assert_eq!(v["model"], "claude-opus-4-8");
        assert_eq!(v["attempts"], 2);
        assert_eq!(v["started_at"], 100);
        assert_eq!(v["settled_at"], 110);
        assert_eq!(v["outcome"], "verdict:reject");
        assert_eq!(v["input_tokens"], 50);
        assert_eq!(v["output_tokens"], 12);
        // round-trips back intact.
        let back: InvocationRow = serde_json::from_value(v).unwrap();
        assert_eq!(back, row);
    }

    #[test]
    fn error_class_maps_every_runner_error_variant() {
        assert_eq!(ErrorClass::of(&RunnerError::RateLimited("x".into())).as_str(), "rate_limited");
        assert_eq!(ErrorClass::of(&RunnerError::Spawn("x".into())).as_str(), "spawn");
        assert_eq!(ErrorClass::of(&RunnerError::NoResult).as_str(), "no_result");
        assert_eq!(ErrorClass::of(&RunnerError::Other("x".into())).as_str(), "other");
        assert_eq!(ErrorClass::of(&RunnerError::ModelUnavailable("x".into())).as_str(), "model_unavailable");
    }
}
