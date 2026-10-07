//! The latest plan Utilization reading, persisted in the one-row
//! `utilization_state` table so the meter has a value across restarts.

use agent_bus_core::UtilizationReading;
use sqlx::SqlitePool;

/// What the last poll left behind. `reading` is the last *good* reading; a
/// failed poll keeps it and records the error.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StoredUtilization {
    pub reading: Option<UtilizationReading>,
    pub last_ok_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    pub last_error: Option<String>,
}

impl StoredUtilization {
    /// The last poll succeeded.
    pub fn available(&self) -> bool {
        self.last_attempt_at.is_some() && self.last_error.is_none()
    }
}

pub struct UtilizationStore {
    pool: SqlitePool,
}

impl UtilizationStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn record_ok(&self, reading: &UtilizationReading, now: i64) {
        let json = serde_json::to_string(reading).unwrap_or_default();
        if let Err(e) = sqlx::query(
            "UPDATE utilization_state SET reading_json = ?, last_ok_at = ?, last_attempt_at = ?, last_error = NULL WHERE id = 1",
        )
        .bind(json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        {
            eprintln!("usage_telemetry: failed to store utilization: {e}");
        }
    }

    pub async fn record_err(&self, err: &str, now: i64) {
        if let Err(e) = sqlx::query("UPDATE utilization_state SET last_attempt_at = ?, last_error = ? WHERE id = 1")
            .bind(now)
            .bind(err)
            .execute(&self.pool)
            .await
        {
            eprintln!("usage_telemetry: failed to store utilization error: {e}");
        }
    }

    pub async fn load(&self) -> StoredUtilization {
        let row: Option<(Option<String>, Option<i64>, Option<i64>, Option<String>)> = sqlx::query_as(
            "SELECT reading_json, last_ok_at, last_attempt_at, last_error FROM utilization_state WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();
        match row {
            Some((json, last_ok_at, last_attempt_at, last_error)) => StoredUtilization {
                reading: json.and_then(|j| serde_json::from_str(&j).ok()),
                last_ok_at,
                last_attempt_at,
                last_error,
            },
            None => StoredUtilization::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{LimitKind, LimitReading};
    use sqlx::sqlite::SqlitePoolOptions;

    async fn store() -> UtilizationStore {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/005_usage.sql")).execute(&pool).await.unwrap();
        sqlx::query("ALTER TABLE worker_usage_log ADD COLUMN cost_micros INTEGER").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE utilization_state (id INTEGER PRIMARY KEY CHECK (id = 1), reading_json TEXT, last_ok_at INTEGER, last_attempt_at INTEGER, last_error TEXT)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO utilization_state (id) VALUES (1)").execute(&pool).await.unwrap();
        UtilizationStore::new(pool)
    }

    fn reading(pct: f64) -> UtilizationReading {
        UtilizationReading { observed_at: 10, limits: vec![LimitReading { kind: LimitKind::Session, utilization_pct: pct, resets_at: 99 }] }
    }

    #[tokio::test]
    async fn empty_store_has_no_reading_and_is_unavailable() {
        let s = store().await.load().await;
        assert!(s.reading.is_none());
        assert!(!s.available());
    }

    #[tokio::test]
    async fn ok_then_err_keeps_the_last_good_reading_but_is_unavailable() {
        let st = store().await;
        st.record_ok(&reading(41.0), 10).await;
        assert!(st.load().await.available());
        st.record_err("usage query timed out", 20).await;
        let s = st.load().await;
        assert_eq!(s.reading, Some(reading(41.0)));
        assert_eq!(s.last_ok_at, Some(10));
        assert_eq!(s.last_attempt_at, Some(20));
        assert_eq!(s.last_error.as_deref(), Some("usage query timed out"));
        assert!(!s.available());
    }
}
