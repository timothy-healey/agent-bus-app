//! The WorkerPool aggregate's persistence. A Worker row records which team a
//! worker belongs to and (when busy) which task_id it holds — the cross-
//! reference to the Task aggregate (context-map.md: "two aggregates, joined by
//! reference"). v1 invocations are ephemeral so pid stays NULL.

use agent_bus_core::TeamId;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worker {
    pub id: String,
    pub team_id: TeamId,
    pub task_id: Option<String>,
    pub pid: Option<i64>,
    pub started_at: i64,
}

impl Worker {
    pub fn new(team_id: TeamId, now_unix: i64) -> Self {
        Self {
            id: format!("w-{}", uuid::Uuid::new_v4()),
            team_id,
            task_id: None,
            pid: None,
            started_at: now_unix,
        }
    }
}

#[derive(Debug, Error)]
pub enum WorkerStoreError {
    #[error("worker not found: {0}")]
    NotFound(String),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub struct WorkerStore {
    pool: SqlitePool,
}

impl WorkerStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, w: &Worker) -> Result<(), WorkerStoreError> {
        sqlx::query("INSERT INTO workers (id, team_id, task_id, pid, started_at) VALUES (?,?,?,?,?)")
            .bind(&w.id)
            .bind(&w.team_id.0)
            .bind(&w.task_id)
            .bind(w.pid)
            .bind(w.started_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Set (or clear, with None) the task a worker currently holds.
    pub async fn set_task(&self, worker_id: &str, task_id: Option<&str>) -> Result<(), WorkerStoreError> {
        let res = sqlx::query("UPDATE workers SET task_id = ? WHERE id = ?")
            .bind(task_id)
            .bind(worker_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(WorkerStoreError::NotFound(worker_id.to_string()));
        }
        Ok(())
    }

    pub async fn count_for_team(&self, team_id: &TeamId) -> Result<u32, WorkerStoreError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM workers WHERE team_id = ?")
            .bind(&team_id.0)
            .fetch_one(&self.pool)
            .await?;
        Ok(n as u32)
    }

    pub async fn remove(&self, worker_id: &str) -> Result<(), WorkerStoreError> {
        let res = sqlx::query("DELETE FROM workers WHERE id = ?")
            .bind(worker_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(WorkerStoreError::NotFound(worker_id.to_string()));
        }
        Ok(())
    }

    /// Clear all workers (startup reset — v1 workers are ephemeral, like
    /// invocations; the pool rebuilds them).
    pub async fn clear_all(&self) -> Result<u64, WorkerStoreError> {
        let res = sqlx::query("DELETE FROM workers").execute(&self.pool).await?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_and_count() {
        let store = WorkerStore::new(fresh_pool().await);
        let team = TeamId("research".into());
        store.insert(&Worker::new(team.clone(), 100)).await.unwrap();
        store.insert(&Worker::new(team.clone(), 101)).await.unwrap();
        assert_eq!(store.count_for_team(&team).await.unwrap(), 2);
        assert_eq!(store.count_for_team(&TeamId("other".into())).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn set_and_clear_task() {
        let store = WorkerStore::new(fresh_pool().await);
        let w = Worker::new(TeamId("research".into()), 100);
        store.insert(&w).await.unwrap();
        store.set_task(&w.id, Some("T-1")).await.unwrap();
        store.set_task(&w.id, None).await.unwrap();
        assert!(matches!(store.set_task("nope", None).await, Err(WorkerStoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn remove_and_clear_all() {
        let store = WorkerStore::new(fresh_pool().await);
        let team = TeamId("research".into());
        let w = Worker::new(team.clone(), 100);
        store.insert(&w).await.unwrap();
        store.remove(&w.id).await.unwrap();
        assert_eq!(store.count_for_team(&team).await.unwrap(), 0);
        store.insert(&Worker::new(team.clone(), 1)).await.unwrap();
        assert_eq!(store.clear_all().await.unwrap(), 1);
    }
}
