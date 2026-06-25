//! LiveProcessStore — durable record of spawned `claude` process groups so an
//! UNCLEAN app death can be reaped on the next boot, BEFORE recovery re-queues
//! the task. A composition-root concern (the runtime crate stays unaware),
//! mirroring the persisted-brake pattern. Unix-only reap; the record itself is
//! cross-platform.

use sqlx::SqlitePool;

pub struct LiveProcessStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveProcess {
    pub pgid: i64,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub started_ts: i64,
}

impl LiveProcessStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(
        &self,
        pgid: i64,
        run_id: Option<&str>,
        task_id: Option<&str>,
        started_ts: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT OR REPLACE INTO live_processes (pgid, run_id, task_id, started_ts) VALUES (?,?,?,?)",
        )
        .bind(pgid)
        .bind(run_id)
        .bind(task_id)
        .bind(started_ts)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, pgid: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM live_processes WHERE pgid = ?")
            .bind(pgid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<LiveProcess>, sqlx::Error> {
        let rows: Vec<(i64, Option<String>, Option<String>, i64)> =
            sqlx::query_as("SELECT pgid, run_id, task_id, started_ts FROM live_processes")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(pgid, run_id, task_id, started_ts)| LiveProcess {
                pgid,
                run_id,
                task_id,
                started_ts,
            })
            .collect())
    }

    pub async fn clear_all(&self) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query("DELETE FROM live_processes")
            .execute(&self.pool)
            .await?
            .rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../migrations/013_lifecycle_hardening.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_list_delete_round_trip() {
        let store = LiveProcessStore::new(fresh_pool().await);
        store.insert(1234, Some("R-1"), Some("T-1"), 500).await.unwrap();
        let all = store.list().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].pgid, 1234);
        assert_eq!(all[0].started_ts, 500);
        store.delete(1234).await.unwrap();
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn clear_all_empties_the_table() {
        let store = LiveProcessStore::new(fresh_pool().await);
        store.insert(1, None, None, 1).await.unwrap();
        store.insert(2, None, None, 2).await.unwrap();
        assert_eq!(store.clear_all().await.unwrap(), 2);
        assert!(store.list().await.unwrap().is_empty());
    }
}
