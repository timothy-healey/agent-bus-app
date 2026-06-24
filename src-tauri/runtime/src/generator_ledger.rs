//! GeneratorLedger — SQLite persistence for the generator (source-stage) found-key
//! ledger (Runtime redesign ④a).
//!
//! Per `(run_id, source-stage)` it records the set of candidate keys the
//! generator has already produced this run. Append-only; it is the dedup +
//! dry-detection source of truth. The generator is handed `found_keys` each pass
//! and returns only new keys; `record_keys` uses `INSERT OR IGNORE` against the
//! `(run_id, stage, candidate_key)` PK so re-recording a key is a no-op, and the
//! number of rows actually inserted tells the caller how many keys were NEW (a
//! pass that inserts zero contributes to the generator-dry signal).
//!
//! Additive: not wired into the existing single-task pool — that is ④c.

use sqlx::SqlitePool;
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GeneratorLedgerError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub struct GeneratorLedger {
    pool: SqlitePool,
}

impl GeneratorLedger {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Record candidate keys for `(run_id, stage)`. INSERT OR IGNORE dedups
    /// against the PK, so re-recording a known key inserts 0 rows. Returns the
    /// number of rows actually inserted = how many of `keys` were NEW.
    pub async fn record_keys(
        &self,
        run_id: &str,
        stage: &str,
        keys: &[String],
    ) -> Result<u64, GeneratorLedgerError> {
        if keys.is_empty() {
            return Ok(0);
        }
        let mut inserted = 0u64;
        // One statement per key keeps rows_affected exact per key (a multi-row
        // VALUES INSERT OR IGNORE reports the total but is fine too; per-key keeps
        // it simple and exact, and the batches here are bounded by store capacity).
        for key in keys {
            let res = sqlx::query(
                "INSERT OR IGNORE INTO generator_ledger (run_id, stage, candidate_key)
                 VALUES (?,?,?)",
            )
            .bind(run_id)
            .bind(stage)
            .bind(key)
            .execute(&self.pool)
            .await?;
            inserted += res.rows_affected();
        }
        Ok(inserted)
    }

    /// The set of candidate keys already found for `(run_id, stage)` — the
    /// already-found set handed to the generator each pass.
    pub async fn found_keys(
        &self,
        run_id: &str,
        stage: &str,
    ) -> Result<HashSet<String>, GeneratorLedgerError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT candidate_key FROM generator_ledger WHERE run_id = ? AND stage = ?",
        )
        .bind(run_id)
        .bind(stage)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(k,)| k).collect())
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
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/012_runtime_stores.sql")).execute(&pool).await.unwrap();
        pool
    }

    fn keys(ks: &[&str]) -> Vec<String> {
        ks.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn record_returns_count_of_new_keys() {
        let led = GeneratorLedger::new(fresh_pool().await);
        let n = led.record_keys("R1", "research", &keys(&["a", "b", "c"])).await.unwrap();
        assert_eq!(n, 3, "all three are new");
    }

    #[tokio::test]
    async fn re_recording_a_key_inserts_zero() {
        let led = GeneratorLedger::new(fresh_pool().await);
        led.record_keys("R1", "research", &keys(&["a", "b"])).await.unwrap();
        // re-record one known + one new
        let n = led.record_keys("R1", "research", &keys(&["a", "c"])).await.unwrap();
        assert_eq!(n, 1, "only 'c' is new; 'a' is a dedup no-op");
        // a fully-known batch is the dry signal (0 new)
        let dry = led.record_keys("R1", "research", &keys(&["a", "b", "c"])).await.unwrap();
        assert_eq!(dry, 0, "nothing new -> generator dry");
    }

    #[tokio::test]
    async fn found_keys_returns_the_set() {
        let led = GeneratorLedger::new(fresh_pool().await);
        led.record_keys("R1", "research", &keys(&["a", "b", "c"])).await.unwrap();
        let found = led.found_keys("R1", "research").await.unwrap();
        assert_eq!(found, ["a", "b", "c"].iter().map(|s| s.to_string()).collect());
    }

    #[tokio::test]
    async fn found_keys_scoped_by_run_and_stage() {
        let led = GeneratorLedger::new(fresh_pool().await);
        led.record_keys("R1", "research", &keys(&["a"])).await.unwrap();
        led.record_keys("R2", "research", &keys(&["b"])).await.unwrap();
        led.record_keys("R1", "other", &keys(&["c"])).await.unwrap();
        assert_eq!(led.found_keys("R1", "research").await.unwrap(), ["a".to_string()].into_iter().collect());
        assert!(led.found_keys("R9", "research").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_batch_is_a_no_op() {
        let led = GeneratorLedger::new(fresh_pool().await);
        assert_eq!(led.record_keys("R1", "research", &[]).await.unwrap(), 0);
    }
}
