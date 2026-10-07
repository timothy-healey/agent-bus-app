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
        // Validate the reading for non-finite utilization_pct
        for limit in &reading.limits {
            if !limit.utilization_pct.is_finite() {
                self.record_err("usage reading invalid: non-finite utilization", now).await;
                return;
            }
        }

        // Attempt to serialize the reading
        let json = match serde_json::to_string(reading) {
            Ok(j) => j,
            Err(e) => {
                self.record_err(&format!("usage reading could not be stored: {e}"), now).await;
                return;
            }
        };

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
        let row: Result<Option<(Option<String>, Option<i64>, Option<i64>, Option<String>)>, _> = sqlx::query_as(
            "SELECT reading_json, last_ok_at, last_attempt_at, last_error FROM utilization_state WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await;

        let row = match row {
            Ok(r) => r,
            Err(e) => {
                eprintln!("usage_telemetry: usage store unreadable: {e}");
                return StoredUtilization {
                    reading: None,
                    last_ok_at: None,
                    last_attempt_at: None,
                    last_error: Some(format!("usage store unreadable: {e}")),
                };
            }
        };

        match row {
            Some((json, last_ok_at, last_attempt_at, last_error)) => {
                let reading = match json {
                    Some(ref j) if !j.is_empty() => {
                        match serde_json::from_str::<UtilizationReading>(j) {
                            Ok(r) => Some(r),
                            Err(e) => {
                                eprintln!("usage_telemetry: stored usage reading unreadable: {e}");
                                return StoredUtilization {
                                    reading: None,
                                    last_ok_at,
                                    last_attempt_at,
                                    last_error: Some("stored usage reading unreadable".to_string()),
                                };
                            }
                        }
                    }
                    _ => None,
                };

                StoredUtilization {
                    reading,
                    last_ok_at,
                    last_attempt_at,
                    last_error,
                }
            }
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

    #[tokio::test]
    async fn db_error_surfaces_as_last_error() {
        let st = store().await;
        // Drop the utilization_state table to cause a query error
        sqlx::query("DROP TABLE utilization_state").execute(&st.pool).await.unwrap();
        let s = st.load().await;
        assert!(s.reading.is_none());
        assert!(s.last_error.is_some());
        assert!(s.last_error.as_ref().unwrap().starts_with("usage store unreadable:"));
        assert!(!s.available());
    }

    #[tokio::test]
    async fn corrupt_stored_reading_surfaces_as_error() {
        let st = store().await;
        // Store a good reading first
        st.record_ok(&reading(50.0), 10).await;
        // Corrupt the stored JSON
        sqlx::query("UPDATE utilization_state SET reading_json = 'garbage' WHERE id = 1")
            .execute(&st.pool)
            .await
            .unwrap();
        let s = st.load().await;
        assert!(s.reading.is_none());
        assert_eq!(s.last_error.as_deref(), Some("stored usage reading unreadable"));
        assert!(!s.available());
    }

    #[tokio::test]
    async fn nan_reading_is_rejected() {
        let st = store().await;
        // Store a good reading first
        st.record_ok(&reading(50.0), 10).await;
        assert!(st.load().await.available());

        // Try to store a reading with NaN utilization_pct
        st.record_ok(&reading(f64::NAN), 20).await;

        let s = st.load().await;
        // The good reading should still be there
        assert_eq!(s.reading, Some(reading(50.0)));
        assert_eq!(s.last_ok_at, Some(10));
        // But the store should now be unavailable with an error
        assert_eq!(s.last_error.as_deref(), Some("usage reading invalid: non-finite utilization"));
        assert!(!s.available());
    }

    #[tokio::test]
    async fn recovery_from_error() {
        let st = store().await;
        // Record an error
        st.record_err("temporary failure", 10).await;
        let s = st.load().await;
        assert!(!s.available());

        // Recover with a good reading
        st.record_ok(&reading(35.0), 20).await;
        let s = st.load().await;
        assert_eq!(s.reading, Some(reading(35.0)));
        assert_eq!(s.last_ok_at, Some(20));
        assert_eq!(s.last_attempt_at, Some(20));
        assert!(s.last_error.is_none());
        assert!(s.available());
    }
}
