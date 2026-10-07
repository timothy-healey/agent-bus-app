//! The Utilization poller's testable parts: the 10s floor between polls and
//! one poll step (fetch from the source, record the outcome in the store).

use agent_bus_core::UtilizationSource;
use std::sync::Arc;
use std::time::{Duration, Instant};
use usage_telemetry::utilization_store::UtilizationStore;

/// The minimum gap between two polls.
pub const POLL_FLOOR: Duration = Duration::from_secs(10);

/// How long to wait before the next poll. A trigger that arrives sooner than
/// the floor is deferred to it, not dropped; the first poll runs at once.
pub fn next_poll_delay(last_attempt: Option<Instant>, now: Instant) -> Duration {
    match last_attempt {
        None => Duration::ZERO,
        Some(t) => POLL_FLOOR.saturating_sub(now.saturating_duration_since(t)),
    }
}

/// One poll: fetch off the async runtime and record the outcome. A failure
/// keeps the last good reading and records the error.
pub async fn poll_once(source: Arc<dyn UtilizationSource>, store: &UtilizationStore, now: i64) {
    match tokio::task::spawn_blocking(move || source.fetch()).await {
        Ok(Ok(reading)) => store.record_ok(&reading, now).await,
        Ok(Err(e)) => store.record_err(&e, now).await,
        Err(e) => store.record_err(&format!("usage query failed: {e}"), now).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{LimitKind, LimitReading, UtilizationReading};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Mutex;

    #[test]
    fn first_poll_runs_at_once() {
        assert_eq!(next_poll_delay(None, Instant::now()), Duration::ZERO);
    }

    #[test]
    fn an_early_trigger_is_deferred_to_the_floor() {
        let t = Instant::now();
        assert_eq!(next_poll_delay(Some(t), t + Duration::from_secs(3)), Duration::from_secs(7));
    }

    #[test]
    fn a_trigger_after_the_floor_polls_at_once() {
        let t = Instant::now();
        assert_eq!(next_poll_delay(Some(t), t + Duration::from_secs(10)), Duration::ZERO);
        assert_eq!(next_poll_delay(Some(t), t + Duration::from_secs(45)), Duration::ZERO);
    }

    /// Returns each queued result once, in order.
    struct Scripted(Mutex<Vec<Result<UtilizationReading, String>>>);

    impl UtilizationSource for Scripted {
        fn fetch(&self) -> Result<UtilizationReading, String> {
            self.0.lock().unwrap().remove(0)
        }
    }

    #[tokio::test]
    async fn a_failed_poll_keeps_the_last_good_reading_and_reports_unavailable() {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE utilization_state (id INTEGER PRIMARY KEY CHECK (id = 1), reading_json TEXT, last_ok_at INTEGER, last_attempt_at INTEGER, last_error TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO utilization_state (id) VALUES (1)").execute(&pool).await.unwrap();
        let store = UtilizationStore::new(pool);
        let good = UtilizationReading {
            observed_at: 900,
            limits: vec![LimitReading { kind: LimitKind::Session, utilization_pct: 41.0, resets_at: 5_000 }],
        };
        let source: Arc<dyn UtilizationSource> =
            Arc::new(Scripted(Mutex::new(vec![Ok(good.clone()), Err("usage query timed out".into())])));

        poll_once(source.clone(), &store, 900).await;
        let after_ok = store.load().await;
        assert!(after_ok.available());
        assert_eq!(after_ok.reading.as_ref(), Some(&good));

        poll_once(source, &store, 960).await;
        let after_err = store.load().await;
        assert!(!after_err.available());
        assert_eq!(after_err.last_error.as_deref(), Some("usage query timed out"));
        assert_eq!(after_err.reading, Some(good));
        assert_eq!(after_err.last_ok_at, Some(900));
        assert_eq!(after_err.last_attempt_at, Some(960));
    }
}
