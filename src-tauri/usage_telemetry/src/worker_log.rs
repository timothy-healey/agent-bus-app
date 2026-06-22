//! WorkerUsageStore — persistence for `worker_usage_log` (per-team breakdown +
//! per-task card cost). Also the concrete `UsageSink` (kernel seam) that
//! Runtime's worker loop calls on settle. A telemetry write must never fail a
//! settle, so `record` swallows errors (logs to stderr) — see kernel docs.

use agent_bus_core::{UsageEvent, UsageSink};
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkerUsageError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// One team's slice of the breakdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamUsage {
    pub team_id: String,
    pub tokens: u64,
}

pub struct WorkerUsageStore {
    pub(crate) pool: SqlitePool,
}

impl WorkerUsageStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert one attributed usage row (runner defaults to 'cli' for v1).
    pub async fn insert(&self, e: &UsageEvent) -> Result<(), WorkerUsageError> {
        sqlx::query(
            "INSERT INTO worker_usage_log
               (ts, team_id, task_id, model, runner, input_tokens, output_tokens, cache_creation, cache_read)
             VALUES (?,?,?,?,?,?,?,?,?)",
        )
        .bind(e.ts)
        .bind(&e.team_id.0)
        .bind(e.task_id.as_ref().map(|t| t.0.clone()))
        .bind(&e.model)
        .bind("cli")
        .bind(e.input_tokens as i64)
        .bind(e.output_tokens as i64)
        .bind(e.cache_creation as i64)
        .bind(e.cache_read as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Per-team token totals (input+output) within the window, descending by
    /// tokens. The tooltip's "by team" breakdown.
    pub async fn team_breakdown(&self, since_ts: i64) -> Result<Vec<TeamUsage>, WorkerUsageError> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT team_id, COALESCE(SUM(input_tokens + output_tokens), 0)
             FROM worker_usage_log WHERE ts > ?
             GROUP BY team_id ORDER BY 2 DESC",
        )
        .bind(since_ts)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(team_id, t)| TeamUsage { team_id, tokens: t as u64 }).collect())
    }

    /// Per-task token totals (input+output), all time (cards show lifetime cost).
    pub async fn tokens_by_task(&self) -> Result<Vec<(String, u64)>, WorkerUsageError> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT task_id, COALESCE(SUM(input_tokens + output_tokens), 0)
             FROM worker_usage_log WHERE task_id IS NOT NULL
             GROUP BY task_id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id, t)| (id, t as u64)).collect())
    }

    /// API-runner tokens within the window (the only worker rows that count
    /// toward the window total — D3). v1 has none; returns 0.
    pub async fn api_window_tokens(&self, since_ts: i64) -> Result<u64, WorkerUsageError> {
        let (t,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(input_tokens + output_tokens), 0)
             FROM worker_usage_log WHERE runner = 'api' AND ts > ?",
        )
        .bind(since_ts)
        .fetch_one(&self.pool)
        .await?;
        Ok(t as u64)
    }
}

/// The kernel `UsageSink` impl. Runtime calls this on settle (via Arc). We use a
/// blocking insert on the ambient tokio runtime; failures are logged, never
/// propagated (a usage write must not break a worker settle).
impl UsageSink for WorkerUsageStore {
    fn record(&self, event: UsageEvent) {
        let pool = self.pool.clone();
        // Spawn so `record` stays sync + non-blocking for the caller.
        tokio::spawn(async move {
            let store = WorkerUsageStore { pool };
            if let Err(e) = store.insert(&event).await {
                eprintln!("usage_telemetry: failed to record worker usage: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{TaskId, TeamId};
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/005_usage.sql")).execute(&pool).await.unwrap();
        pool
    }

    fn ev(ts: i64, team: &str, task: Option<&str>, inp: u64, out: u64) -> UsageEvent {
        UsageEvent {
            ts,
            team_id: TeamId(team.into()),
            task_id: task.map(|t| TaskId(t.into())),
            model: "claude-opus-4-7".into(),
            input_tokens: inp,
            output_tokens: out,
            cache_creation: 0,
            cache_read: 0,
        }
    }

    #[tokio::test]
    async fn breakdown_sums_per_team_and_respects_window() {
        let store = WorkerUsageStore::new(fresh_pool().await);
        store.insert(&ev(1000, "research", Some("T-1"), 100, 20)).await.unwrap();
        store.insert(&ev(1000, "research", Some("T-1"), 50, 10)).await.unwrap();
        store.insert(&ev(1000, "writers", Some("T-2"), 30, 5)).await.unwrap();
        store.insert(&ev(100, "research", Some("T-0"), 999, 999)).await.unwrap(); // out of window

        let bd = store.team_breakdown(500).await.unwrap();
        assert_eq!(bd[0], TeamUsage { team_id: "research".into(), tokens: 180 });
        assert_eq!(bd[1], TeamUsage { team_id: "writers".into(), tokens: 35 });
    }

    #[tokio::test]
    async fn tokens_by_task_aggregates_all_time() {
        let store = WorkerUsageStore::new(fresh_pool().await);
        store.insert(&ev(1, "research", Some("T-1"), 100, 20)).await.unwrap();
        store.insert(&ev(2, "writers", Some("T-1"), 30, 5)).await.unwrap();
        let mut by = store.tokens_by_task().await.unwrap();
        by.sort();
        assert_eq!(by, vec![("T-1".to_string(), 155)]);
    }

    #[tokio::test]
    async fn api_window_tokens_is_zero_for_cli_rows() {
        let store = WorkerUsageStore::new(fresh_pool().await);
        store.insert(&ev(1000, "research", Some("T-1"), 100, 20)).await.unwrap();
        assert_eq!(store.api_window_tokens(0).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn sink_record_persists_the_event() {
        let store = WorkerUsageStore::new(fresh_pool().await);
        let e = ev(1000, "research", Some("T-9"), 7, 3);
        // call through the trait object, as Runtime would
        let sink: std::sync::Arc<dyn UsageSink> = std::sync::Arc::new(WorkerUsageStore::new(store.pool.clone()));
        sink.record(e);
        // record spawns; give it a tick to land
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let by = store.tokens_by_task().await.unwrap();
        assert_eq!(by, vec![("T-9".to_string(), 10)]);
    }
}
