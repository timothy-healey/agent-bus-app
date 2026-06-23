//! TaskStore — SQLite persistence for the Task aggregate. The claim is atomic:
//! a conditional UPDATE that flips exactly one queued row to running and tells
//! the caller whether it won (spec: "atomic UPDATE state=running").

use crate::task::{Task, TaskState};
use agent_bus_core::TaskId;
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TaskStoreError {
    #[error("task not found: {0}")]
    NotFound(TaskId),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("corrupt state string in db: {0}")]
    BadState(String),
}

type Row = (
    String, String, String, String, Option<String>, Option<String>,
    String, String, i64, Option<String>, Option<String>, i64, i64,
    Option<String>, Option<String>, Option<String>,
);

pub struct TaskStore {
    pool: SqlitePool,
}

impl TaskStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, task: &Task) -> Result<(), TaskStoreError> {
        sqlx::query(
            "INSERT INTO tasks (id, project_id, pipeline, topic, target_repo, target_scope,
             current_stage, state, attempts, parent_artifact, review_artifact, created_at, updated_at,
             group_id, lane, join_target)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&task.id.0)
        .bind(&task.project_id)
        .bind(&task.pipeline)
        .bind(&task.topic)
        .bind(&task.target_repo)
        .bind(&task.target_scope)
        .bind(&task.current_stage)
        .bind(task.state.as_str())
        .bind(task.attempts as i64)
        .bind(&task.parent_artifact)
        .bind(&task.review_artifact)
        .bind(task.created_at)
        .bind(task.updated_at)
        .bind(&task.group_id)
        .bind(&task.lane)
        .bind(&task.join_target)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    fn row_to_task(r: Row) -> Result<Task, TaskStoreError> {
        let state = TaskState::parse(&r.7).ok_or_else(|| TaskStoreError::BadState(r.7.clone()))?;
        Ok(Task {
            id: TaskId(r.0),
            project_id: r.1,
            pipeline: r.2,
            topic: r.3,
            target_repo: r.4,
            target_scope: r.5,
            current_stage: r.6,
            state,
            attempts: r.8 as u32,
            parent_artifact: r.9,
            review_artifact: r.10,
            created_at: r.11,
            updated_at: r.12,
            group_id: r.13,
            lane: r.14,
            join_target: r.15,
        })
    }

    const SELECT: &'static str =
        "SELECT id, project_id, pipeline, topic, target_repo, target_scope, current_stage,
         state, attempts, parent_artifact, review_artifact, created_at, updated_at,
         group_id, lane, join_target FROM tasks";

    pub async fn get(&self, id: &TaskId) -> Result<Task, TaskStoreError> {
        let row = sqlx::query_as::<_, Row>(&format!("{} WHERE id = ?", Self::SELECT))
            .bind(&id.0)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => Self::row_to_task(r),
            None => Err(TaskStoreError::NotFound(id.clone())),
        }
    }

    pub async fn list_by_state(&self, state: TaskState) -> Result<Vec<Task>, TaskStoreError> {
        let rows = sqlx::query_as::<_, Row>(&format!(
            "{} WHERE state = ? ORDER BY created_at ASC",
            Self::SELECT
        ))
        .bind(state.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_task).collect()
    }

    /// Persist the whole task row (state, stage, attempts, artifacts, updated_at).
    pub async fn update(&self, task: &Task) -> Result<(), TaskStoreError> {
        let res = sqlx::query(
            "UPDATE tasks SET current_stage=?, state=?, attempts=?, parent_artifact=?,
             review_artifact=?, updated_at=?, group_id=?, lane=?, join_target=? WHERE id=?",
        )
        .bind(&task.current_stage)
        .bind(task.state.as_str())
        .bind(task.attempts as i64)
        .bind(&task.parent_artifact)
        .bind(&task.review_artifact)
        .bind(task.updated_at)
        .bind(&task.group_id)
        .bind(&task.lane)
        .bind(&task.join_target)
        .bind(&task.id.0)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(TaskStoreError::NotFound(task.id.clone()));
        }
        Ok(())
    }

    /// Atomically claim the oldest queued task for a stage: flip it to running.
    /// Returns Some(task) if this caller won a row, None if none were available.
    /// The conditional UPDATE guarantees at most one worker wins a given row.
    pub async fn claim_next_for_stage(
        &self,
        stage: &str,
        now_unix: i64,
    ) -> Result<Option<Task>, TaskStoreError> {
        // Find the oldest queued candidate, then conditionally claim it. The
        // WHERE state='queued' in the UPDATE is the atomic guard: a racing
        // worker that lost sees rows_affected == 0 and retries.
        let candidate = sqlx::query_as::<_, (String,)>(
            "SELECT id FROM tasks WHERE current_stage = ? AND state = 'queued'
             ORDER BY created_at ASC LIMIT 1",
        )
        .bind(stage)
        .fetch_optional(&self.pool)
        .await?;

        let Some((id,)) = candidate else { return Ok(None) };

        let res = sqlx::query(
            "UPDATE tasks SET state='running', updated_at=? WHERE id=? AND state='queued'",
        )
        .bind(now_unix)
        .bind(&id)
        .execute(&self.pool)
        .await?;

        if res.rows_affected() == 0 {
            // Lost the race; caller may retry.
            return Ok(None);
        }
        Ok(Some(self.get(&TaskId(id)).await?))
    }

    /// Early-cancel (P2): park the still-active lane tasks of a fan-out group so
    /// queued ones are never claimed and run wastefully once the group has
    /// already resolved (to needs-human). Transitions tasks of `group_id` whose
    /// state is `queued` or `revising` to `done`, parked at their `join_target`
    /// (mirrors how the barrier parks a settled lane task). A `running` lane is
    /// deliberately left alone (DD5: never kill a live invocation — its later
    /// settle hits the already-`completed` group guard and no-ops). Returns the
    /// count parked. Like `claim_next_for_stage`, this is a direct conditional
    /// UPDATE — the early-cancel park does not route through Task::transition_to,
    /// consistent with the barrier's existing direct `state = Done` park.
    pub async fn cancel_outstanding_lanes(
        &self,
        group_id: &str,
        now_unix: i64,
    ) -> Result<u64, TaskStoreError> {
        let res = sqlx::query(
            "UPDATE tasks
             SET state='done',
                 current_stage=COALESCE(join_target, current_stage),
                 updated_at=?
             WHERE group_id=? AND state IN ('queued','revising')",
        )
        .bind(now_unix)
        .bind(group_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    /// Crash recovery (spec F4): release any task stuck in `running` back to
    /// `queued` on startup, since v1 keeps no live invocation record. Returns
    /// the count released.
    pub async fn release_orphaned_running(&self, now_unix: i64) -> Result<u64, TaskStoreError> {
        let res = sqlx::query(
            "UPDATE tasks SET state='queued', updated_at=? WHERE state='running'",
        )
        .bind(now_unix)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        // FK enforcement is off here so the tests can insert tasks without a
        // backing projects row (the plan's test bodies use project_id "p").
        // Matches the raw-sqlite3 migration verify, which also has FKs off.
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/006_fanout.sql")).execute(&pool).await.unwrap();
        pool
    }

    fn task(stage: &str) -> Task {
        Task::injected("p".into(), "pipe".into(), stage.into(), "topic".into(), None, 100)
    }

    #[tokio::test]
    async fn insert_get_round_trip() {
        let store = TaskStore::new(fresh_pool().await);
        let t = task("research");
        store.insert(&t).await.unwrap();
        assert_eq!(store.get(&t.id).await.unwrap(), t);
    }

    #[tokio::test]
    async fn get_missing_is_not_found() {
        let store = TaskStore::new(fresh_pool().await);
        assert!(matches!(store.get(&TaskId("nope".into())).await, Err(TaskStoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn claim_flips_queued_to_running_and_is_exclusive() {
        let store = TaskStore::new(fresh_pool().await);
        let t = task("research");
        store.insert(&t).await.unwrap();

        let claimed = store.claim_next_for_stage("research", 200).await.unwrap().unwrap();
        assert_eq!(claimed.state, TaskState::Running);
        assert_eq!(claimed.id, t.id);

        // second claim finds nothing queued
        assert!(store.claim_next_for_stage("research", 201).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn claim_returns_none_for_empty_stage() {
        let store = TaskStore::new(fresh_pool().await);
        assert!(store.claim_next_for_stage("research", 1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_by_state_filters() {
        let store = TaskStore::new(fresh_pool().await);
        store.insert(&task("research")).await.unwrap();
        store.insert(&task("research")).await.unwrap();
        assert_eq!(store.list_by_state(TaskState::Queued).await.unwrap().len(), 2);
        assert_eq!(store.list_by_state(TaskState::Running).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn update_persists_state_and_attempts() {
        let store = TaskStore::new(fresh_pool().await);
        let mut t = task("research");
        store.insert(&t).await.unwrap();
        t.transition_to(TaskState::Running, 200).unwrap();
        t.current_stage = "spec-writers".into();
        store.update(&t).await.unwrap();
        let reloaded = store.get(&t.id).await.unwrap();
        assert_eq!(reloaded.state, TaskState::Running);
        assert_eq!(reloaded.current_stage, "spec-writers");
    }

    #[tokio::test]
    async fn insert_get_round_trips_lane_fields() {
        let store = TaskStore::new(fresh_pool().await);
        let parent = task("research");
        let sib = Task::forked(&parent, "lane-a", "G-1", "join-1", 500);
        store.insert(&sib).await.unwrap();
        let back = store.get(&sib.id).await.unwrap();
        assert_eq!(back.group_id.as_deref(), Some("G-1"));
        assert_eq!(back.lane.as_deref(), Some("lane-a"));
        assert_eq!(back.join_target.as_deref(), Some("join-1"));
    }

    #[tokio::test]
    async fn linear_task_has_null_lane_fields() {
        let store = TaskStore::new(fresh_pool().await);
        let t = task("research");
        store.insert(&t).await.unwrap();
        let back = store.get(&t.id).await.unwrap();
        assert_eq!(back.group_id, None);
        assert_eq!(back.lane, None);
        assert_eq!(back.join_target, None);
    }

    #[tokio::test]
    async fn cancel_outstanding_lanes_parks_queued_and_revising_not_running_or_done() {
        let store = TaskStore::new(fresh_pool().await);
        let parent = task("entry");
        // queued lane task in group G-1
        let queued = Task::forked(&parent, "lane-a", "G-1", "join-1", 500);
        store.insert(&queued).await.unwrap();
        // a running lane task in the same group (must be left alone — DD5)
        let mut running = Task::forked(&parent, "lane-b", "G-1", "join-1", 500);
        running.state = TaskState::Running;
        store.insert(&running).await.unwrap();
        // a lane task in a DIFFERENT group (must be untouched)
        let other = Task::forked(&parent, "lane-c", "G-2", "join-2", 500);
        store.insert(&other).await.unwrap();

        let cancelled = store.cancel_outstanding_lanes("G-1", 900).await.unwrap();
        assert_eq!(cancelled, 1, "only the queued lane in G-1 is parked");

        // queued -> Done, parked at its join_target
        let q = store.get(&queued.id).await.unwrap();
        assert_eq!(q.state, TaskState::Done);
        assert_eq!(q.current_stage, "join-1");
        // running is left to finish (DD5)
        assert_eq!(store.get(&running.id).await.unwrap().state, TaskState::Running);
        // other group untouched
        assert_eq!(store.get(&other.id).await.unwrap().state, TaskState::Queued);
    }

    #[tokio::test]
    async fn release_orphaned_running_requeues() {
        let store = TaskStore::new(fresh_pool().await);
        let t = task("research");
        store.insert(&t).await.unwrap();
        store.claim_next_for_stage("research", 200).await.unwrap().unwrap();
        let released = store.release_orphaned_running(300).await.unwrap();
        assert_eq!(released, 1);
        assert_eq!(store.get(&t.id).await.unwrap().state, TaskState::Queued);
    }
}
