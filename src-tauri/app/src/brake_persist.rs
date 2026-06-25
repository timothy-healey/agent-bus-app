//! BrakeStore — durable brake state (LH6). The runtime `Brake` aggregate stays
//! persistence-unaware; the root writes this row on every set_on/set_off and
//! reads it (reason-aware) on boot. Lives in `app` (a composition-root concern).

use sqlx::SqlitePool;
use usage_telemetry::brake_policy::AUTO_METER_REASON;

/// Does a brake reason represent operator/reactive INTENT that must survive a
/// reboot (come up braked, no auto-resume)? An auto-meter brake is a derived,
/// self-releasing function of a usage window — it must NOT be restored
/// authoritatively (the next sweep re-derives it from fresh telemetry).
pub fn persists_across_reboot(reason: Option<&str>) -> bool {
    !matches!(reason, Some(AUTO_METER_REASON))
}

pub struct BrakeStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedBrake {
    pub on: bool,
    pub reason: Option<String>,
    pub ts: i64,
}

impl BrakeStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn save(
        &self,
        on: bool,
        reason: Option<&str>,
        ts: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE brake_state SET on_flag = ?, reason = ?, ts = ? WHERE id = 1")
            .bind(on as i64)
            .bind(reason)
            .bind(ts)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn load(&self) -> Result<PersistedBrake, sqlx::Error> {
        let (on_flag, reason, ts): (i64, Option<String>, i64) =
            sqlx::query_as("SELECT on_flag, reason, ts FROM brake_state WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        Ok(PersistedBrake {
            on: on_flag != 0,
            reason,
            ts,
        })
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

    #[test]
    fn manual_reasons_persist_auto_meter_does_not() {
        assert!(persists_across_reboot(Some("manual")));
        assert!(persists_across_reboot(Some("rate-limit")));
        assert!(persists_across_reboot(None));
        assert!(!persists_across_reboot(Some(AUTO_METER_REASON)));
    }

    #[tokio::test]
    async fn save_then_load_round_trip() {
        let store = BrakeStore::new(fresh_pool().await);
        store.save(true, Some("manual"), 500).await.unwrap();
        let p = store.load().await.unwrap();
        assert_eq!(
            p,
            PersistedBrake {
                on: true,
                reason: Some("manual".into()),
                ts: 500
            }
        );
        store.save(false, None, 600).await.unwrap();
        assert_eq!(
            store.load().await.unwrap(),
            PersistedBrake {
                on: false,
                reason: None,
                ts: 600
            }
        );
    }
}
