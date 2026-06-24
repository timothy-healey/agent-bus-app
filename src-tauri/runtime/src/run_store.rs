//! RunStore — SQLite persistence for the Run aggregate (Runtime redesign ④a).
//!
//! A Run is one execution of a pipeline: the tree of work-items produced from a
//! single Start. Aggregate root keyed by `run_id`; it owns exactly one invariant
//! — **a run completes exactly once** — enforced by the same conditional-UPDATE
//! single-writer guard the FanOutGroup barrier uses: `UPDATE runs SET completed=1
//! WHERE id=? AND completed=0`. The `rows_affected == 1` is the sole caller that
//! completed the run.
//!
//! The *precondition* for completion (generator dry AND all stores empty AND no
//! running workers) is computed by the caller in ④b/④h; this module is only the
//! single-writer guard + the `generator_dry` flag it reads.
//!
//! Additive: not wired into the existing single-task pool — that is ④b.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RunStoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("run not found: {0}")]
    NotFound(String),
}

/// A Run row (the aggregate root's persisted state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub pipeline: String,
    pub project_id: String,
    pub generator_dry: bool,
    pub completed: bool,
    pub created_at: i64,
}

impl Run {
    /// A fresh, not-yet-dry, not-yet-completed run.
    pub fn new(id: String, pipeline: String, project_id: String, created_at: i64) -> Self {
        Self { id, pipeline, project_id, generator_dry: false, completed: false, created_at }
    }
}

pub struct RunStore {
    pool: SqlitePool,
}

impl RunStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Persist a new run row.
    pub async fn create(&self, run: &Run) -> Result<(), RunStoreError> {
        sqlx::query(
            "INSERT INTO runs (id, pipeline, project_id, generator_dry, completed, created_at)
             VALUES (?,?,?,?,?,?)",
        )
        .bind(&run.id)
        .bind(&run.pipeline)
        .bind(&run.project_id)
        .bind(run.generator_dry as i64)
        .bind(run.completed as i64)
        .bind(run.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load a run by id.
    pub async fn get(&self, run_id: &str) -> Result<Run, RunStoreError> {
        let row = sqlx::query_as::<_, (String, String, String, i64, i64, i64)>(
            "SELECT id, pipeline, project_id, generator_dry, completed, created_at
             FROM runs WHERE id = ?",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| RunStoreError::NotFound(run_id.to_string()))?;
        Ok(Run {
            id: row.0,
            pipeline: row.1,
            project_id: row.2,
            generator_dry: row.3 != 0,
            completed: row.4 != 0,
            created_at: row.5,
        })
    }

    /// Mark the source generator dry (idempotent). A dry generator is one input
    /// to the completion precondition the caller checks before `try_complete`.
    pub async fn set_generator_dry(&self, run_id: &str) -> Result<(), RunStoreError> {
        let res = sqlx::query("UPDATE runs SET generator_dry = 1 WHERE id = ?")
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(RunStoreError::NotFound(run_id.to_string()));
        }
        Ok(())
    }

    /// The newest still-running (not-completed) run for a project, if any. The
    /// worker loops poll this to find the run they should drive: `start_run`
    /// creates a run, the loops pick it up on their next poll and drive its engine
    /// step until it completes. Newest-first so a fresh Start supersedes an older
    /// in-flight run for the same project (v1 drives one active run per project).
    pub async fn latest_active_for_project(
        &self,
        project_id: &str,
    ) -> Result<Option<Run>, RunStoreError> {
        let row = sqlx::query_as::<_, (String, String, String, i64, i64, i64)>(
            "SELECT id, pipeline, project_id, generator_dry, completed, created_at
             FROM runs WHERE project_id = ? AND completed = 0
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| Run {
            id: r.0,
            pipeline: r.1,
            project_id: r.2,
            generator_dry: r.3 != 0,
            completed: r.4 != 0,
            created_at: r.5,
        }))
    }

    /// Every run for a project, newest first (created_at DESC, id DESC). Powers
    /// the frontend run selector (Runtime redesign ④e): the board scopes to one
    /// of these; the topmost not-completed run is the active one. Includes
    /// completed runs so the operator can review past runs.
    pub async fn list_for_project(&self, project_id: &str) -> Result<Vec<Run>, RunStoreError> {
        let rows = sqlx::query_as::<_, (String, String, String, i64, i64, i64)>(
            "SELECT id, pipeline, project_id, generator_dry, completed, created_at
             FROM runs WHERE project_id = ?
             ORDER BY created_at DESC, id DESC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Run {
                id: r.0,
                pipeline: r.1,
                project_id: r.2,
                generator_dry: r.3 != 0,
                completed: r.4 != 0,
                created_at: r.5,
            })
            .collect())
    }

    /// Attempt to complete the run exactly once. The conditional UPDATE is the
    /// guard: `rows_affected == 1` ⇒ this caller is the sole completer; `0` ⇒ the
    /// run was already completed (another caller won, or a re-call). The
    /// completion *precondition* is the caller's responsibility (④b/④h).
    pub async fn try_complete(&self, run_id: &str) -> Result<bool, RunStoreError> {
        let res = sqlx::query("UPDATE runs SET completed = 1 WHERE id = ? AND completed = 0")
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;
    use std::sync::Arc;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/012_runtime_stores.sql")).execute(&pool).await.unwrap();
        pool
    }

    fn run() -> Run {
        Run::new("R1".into(), "pipe".into(), "proj".into(), 100)
    }

    #[tokio::test]
    async fn create_get_round_trip() {
        let store = RunStore::new(fresh_pool().await);
        store.create(&run()).await.unwrap();
        assert_eq!(store.get("R1").await.unwrap(), run());
    }

    #[tokio::test]
    async fn get_missing_is_not_found() {
        let store = RunStore::new(fresh_pool().await);
        assert!(matches!(store.get("nope").await, Err(RunStoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn set_generator_dry_flips_the_flag() {
        let store = RunStore::new(fresh_pool().await);
        store.create(&run()).await.unwrap();
        assert!(!store.get("R1").await.unwrap().generator_dry);
        store.set_generator_dry("R1").await.unwrap();
        assert!(store.get("R1").await.unwrap().generator_dry);
        // idempotent
        store.set_generator_dry("R1").await.unwrap();
        assert!(store.get("R1").await.unwrap().generator_dry);
    }

    #[tokio::test]
    async fn try_complete_succeeds_once_then_no_ops() {
        let store = RunStore::new(fresh_pool().await);
        store.create(&run()).await.unwrap();
        assert!(store.try_complete("R1").await.unwrap(), "first call completes");
        assert!(store.get("R1").await.unwrap().completed);
        assert!(!store.try_complete("R1").await.unwrap(), "second call no-ops");
    }

    #[tokio::test]
    async fn latest_active_for_project_picks_newest_incomplete() {
        let store = RunStore::new(fresh_pool().await);
        // none yet
        assert!(store.latest_active_for_project("proj").await.unwrap().is_none());
        let mut r1 = Run::new("R1".into(), "pipe".into(), "proj".into(), 100);
        r1.created_at = 100;
        store.create(&r1).await.unwrap();
        let mut r2 = Run::new("R2".into(), "pipe".into(), "proj".into(), 200);
        r2.created_at = 200;
        store.create(&r2).await.unwrap();
        // newest incomplete wins
        assert_eq!(store.latest_active_for_project("proj").await.unwrap().unwrap().id, "R2");
        // complete R2 → R1 becomes the active one
        store.try_complete("R2").await.unwrap();
        assert_eq!(store.latest_active_for_project("proj").await.unwrap().unwrap().id, "R1");
        // a different project sees none
        assert!(store.latest_active_for_project("other").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_for_project_returns_newest_first_including_completed() {
        let store = RunStore::new(fresh_pool().await);
        assert!(store.list_for_project("proj").await.unwrap().is_empty());
        let mut r1 = Run::new("R1".into(), "pipe".into(), "proj".into(), 100);
        r1.created_at = 100;
        store.create(&r1).await.unwrap();
        let mut r2 = Run::new("R2".into(), "pipe".into(), "proj".into(), 200);
        r2.created_at = 200;
        store.create(&r2).await.unwrap();
        // a different project's run must not leak in
        store.create(&Run::new("RX".into(), "pipe".into(), "other".into(), 300)).await.unwrap();
        store.try_complete("R1").await.unwrap();
        let runs = store.list_for_project("proj").await.unwrap();
        assert_eq!(runs.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["R2", "R1"]);
        // completed runs are included (the operator reviews past runs)
        assert!(runs.iter().find(|r| r.id == "R1").unwrap().completed);
        assert!(!runs.iter().find(|r| r.id == "R2").unwrap().completed);
    }

    #[tokio::test]
    async fn concurrent_double_complete_yields_exactly_one_winner() {
        // THE CRUX: two callers both attempt completion; the WHERE completed=0
        // guard means exactly one wins.
        let store = Arc::new(RunStore::new(fresh_pool().await));
        store.create(&run()).await.unwrap();
        let s1 = store.clone();
        let s2 = store.clone();
        let h1 = tokio::spawn(async move { s1.try_complete("R1").await.unwrap() });
        let h2 = tokio::spawn(async move { s2.try_complete("R1").await.unwrap() });
        let (a, b) = (h1.await.unwrap(), h2.await.unwrap());
        assert_eq!([a, b].iter().filter(|w| **w).count(), 1, "exactly one completer");
    }
}
