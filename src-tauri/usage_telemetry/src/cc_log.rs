//! CcUsageStore — persistence for `cc_usage_log` (the window-total source, D3).
//! Dedup is enforced by the table's UNIQUE(message_id): re-ingesting the same
//! transcript line is a no-op via INSERT OR IGNORE.

use crate::transcript::CcUsageRecord;
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CcUsageError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub struct CcUsageStore {
    pub(crate) pool: SqlitePool,
}

impl CcUsageStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert one record; duplicates (same message_id) are ignored. Returns true
    /// if the row was new.
    pub async fn insert(&self, r: &CcUsageRecord) -> Result<bool, CcUsageError> {
        let res = sqlx::query(
            "INSERT OR IGNORE INTO cc_usage_log
               (ts, message_id, model, input_tokens, output_tokens, cache_creation, cache_read)
             VALUES (?,?,?,?,?,?,?)",
        )
        .bind(r.ts)
        .bind(&r.message_id)
        .bind(&r.model)
        .bind(r.input_tokens as i64)
        .bind(r.output_tokens as i64)
        .bind(r.cache_creation as i64)
        .bind(r.cache_read as i64)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Ingest many records (a parsed transcript blob), deduping via the unique
    /// index. Returns how many were newly inserted.
    pub async fn ingest(&self, records: &[CcUsageRecord]) -> Result<u64, CcUsageError> {
        let mut new = 0u64;
        for r in records {
            if self.insert(r).await? {
                new += 1;
            }
        }
        Ok(new)
    }

    /// Window total: SUM(input+output) for rows newer than since_ts. This is the
    /// v1 window-total source (D3).
    pub async fn window_tokens(&self, since_ts: i64) -> Result<u64, CcUsageError> {
        let (t,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(input_tokens + output_tokens), 0)
             FROM cc_usage_log WHERE ts > ?",
        )
        .bind(since_ts)
        .fetch_one(&self.pool)
        .await?;
        Ok(t as u64)
    }

    /// Tokens (input+output) in the last `secs` seconds before `now` — the burn
    /// rate's numerator (D7).
    pub async fn recent_tokens(&self, now_ts: i64, secs: i64) -> Result<u64, CcUsageError> {
        self.window_tokens(now_ts - secs).await
    }

    /// The oldest counted event timestamp within the window (for the braked
    /// "↻ reset in" countdown: reset = oldest_ts + window_secs). None if empty.
    pub async fn oldest_in_window(&self, since_ts: i64) -> Result<Option<i64>, CcUsageError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT MIN(ts) FROM cc_usage_log WHERE ts > ?",
        )
        .bind(since_ts)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(t,)| t).filter(|t| *t > 0))
    }

    /// Delete rows older than `before_ts` (bounds table growth; the spec prunes
    /// `now - 2 * window_secs` each sweep). Returns rows removed.
    pub async fn prune(&self, before_ts: i64) -> Result<u64, CcUsageError> {
        let res = sqlx::query("DELETE FROM cc_usage_log WHERE ts < ?")
            .bind(before_ts)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::parse_transcript;
    use crate::transcript::CcUsageRecord;
    use sqlx::sqlite::SqlitePoolOptions;

    const SAMPLE: &str = include_str!("fixtures/transcript-sample.jsonl");

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/005_usage.sql")).execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn ingest_dedupes_on_message_id() {
        let store = CcUsageStore::new(fresh_pool().await);
        let recs = parse_transcript(SAMPLE); // three records, msg_aaa twice
        let new = store.ingest(&recs).await.unwrap();
        assert_eq!(new, 2); // msg_aaa + msg_bbb; the second msg_aaa is ignored
        // re-ingesting the same blob inserts nothing new
        assert_eq!(store.ingest(&recs).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn window_tokens_sums_input_plus_output_in_window() {
        let store = CcUsageStore::new(fresh_pool().await);
        store.ingest(&parse_transcript(SAMPLE)).await.unwrap();
        // msg_aaa: 1200+300=1500 ; msg_bbb: 500+120=620 ; total 2120
        assert_eq!(store.window_tokens(0).await.unwrap(), 2120);
    }

    #[tokio::test]
    async fn oldest_in_window_is_the_min_ts() {
        let store = CcUsageStore::new(fresh_pool().await);
        store.ingest(&parse_transcript(SAMPLE)).await.unwrap();
        let oldest = store.oldest_in_window(0).await.unwrap().unwrap();
        // msg_aaa at 11:14 is earlier than msg_bbb at 11:20
        assert!(oldest > 1_700_000_000);
        // none in an empty window
        assert!(store.oldest_in_window(i64::MAX / 2).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn prune_removes_rows_before_cutoff_keeps_in_window() {
        let store = CcUsageStore::new(fresh_pool().await);
        store.ingest(&parse_transcript(SAMPLE)).await.unwrap(); // ts ~ 1.7e9 (2023+)
        // a row older than the cutoff and a row newer
        store
            .insert(&CcUsageRecord {
                message_id: "old".into(),
                ts: 1_000,
                model: None,
                input_tokens: 10,
                output_tokens: 0,
                cache_creation: 0,
                cache_read: 0,
            })
            .await
            .unwrap();
        let removed = store.prune(2_000).await.unwrap();
        assert_eq!(removed, 1); // only "old" (ts=1000 < 2000)
        // the SAMPLE rows (ts ~1.7e9) survive
        assert!(store.window_tokens(0).await.unwrap() > 0);
        // pruning an empty range removes nothing
        assert_eq!(store.prune(2_000).await.unwrap(), 0);
    }
}
