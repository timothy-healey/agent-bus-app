//! Store — SQLite persistence for the Store aggregate (Runtime redesign ④a).
//!
//! A Store is a capacity-limited buffer holding the work-items waiting for a
//! stage, keyed by `(run_id, stage)`. It owns exactly one invariant —
//! **occupancy never exceeds capacity** — enforced by the same atomic
//! conditional-UPDATE single-writer idiom the Task claim and the FanOutGroup
//! `completed` guard use: `UPDATE … SET occupancy = occupancy + 1 WHERE … AND
//! occupancy < capacity`. The `rows_affected == 1` is the sole winner of a slot;
//! `0` means the store is full (backpressure). No count-then-act race.
//!
//! Additive: nothing here is wired into the existing single-task pool — that is
//! ④b (worker pools, block-before-claim).

use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Persistence + the occupancy<=capacity guard for the Store aggregate.
pub struct StoreRepo {
    pool: SqlitePool,
}

impl StoreRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Idempotently create the store for `(run_id, stage)` with `capacity`.
    /// INSERT OR IGNORE: re-ensuring an existing store is a no-op (it never
    /// resets occupancy or changes capacity).
    pub async fn ensure(
        &self,
        run_id: &str,
        stage: &str,
        capacity: u32,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT OR IGNORE INTO stores (run_id, stage, capacity, occupancy)
             VALUES (?,?,?,0)",
        )
        .bind(run_id)
        .bind(stage)
        .bind(capacity as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Atomically reserve a slot. The conditional UPDATE is the guard: it
    /// increments occupancy only while `occupancy < capacity`, so concurrent
    /// callers can never push occupancy past capacity. Returns `true` if this
    /// caller won a slot, `false` if the store is full (backpressure).
    pub async fn reserve(&self, run_id: &str, stage: &str) -> Result<bool, StoreError> {
        let res = sqlx::query(
            "UPDATE stores SET occupancy = occupancy + 1
             WHERE run_id = ? AND stage = ? AND occupancy < capacity",
        )
        .bind(run_id)
        .bind(stage)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Release a previously-reserved slot (undo on failure / on an item leaving
    /// the store). The `occupancy > 0` guard keeps occupancy from going negative
    /// under any racing release. Returns `true` if a slot was actually freed.
    pub async fn release(&self, run_id: &str, stage: &str) -> Result<bool, StoreError> {
        let res = sqlx::query(
            "UPDATE stores SET occupancy = occupancy - 1
             WHERE run_id = ? AND stage = ? AND occupancy > 0",
        )
        .bind(run_id)
        .bind(stage)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Current occupancy of the store, or `None` if it does not exist.
    pub async fn occupancy(&self, run_id: &str, stage: &str) -> Result<Option<u32>, StoreError> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT occupancy FROM stores WHERE run_id = ? AND stage = ?")
                .bind(run_id)
                .bind(stage)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(o,)| o as u32))
    }

    /// Whether the store is at capacity (a reserve would fail). A missing store
    /// is reported as not-full (it admits work once ensured).
    pub async fn is_full(&self, run_id: &str, stage: &str) -> Result<bool, StoreError> {
        let row: Option<(i64, i64)> =
            sqlx::query_as("SELECT occupancy, capacity FROM stores WHERE run_id = ? AND stage = ?")
                .bind(run_id)
                .bind(stage)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(o, c)| o >= c).unwrap_or(false))
    }

    /// Boot/resume reconciliation: for every store of `run_id`, set `occupancy`
    /// to the count of work-items actually resident at that stage — the truth
    /// from `tasks`. Residency = a work-item parked at `current_stage = <stage>`
    /// in a state that occupies a slot: `queued`, `running`, `gated`, `revising`,
    /// `joining`. (`done` left the store; `needs_human` was routed to the
    /// escalation stage and its slot released.) This clears reservations leaked by
    /// a killed/crashed worker (occupancy incremented at `reserve`, never released)
    /// so a resumed run is not falsely backpressured. Run AFTER
    /// `TaskStore::release_orphaned_running` so just-requeued items are counted.
    pub async fn reconcile_occupancy(&self, run_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE stores
                SET occupancy = (
                    SELECT COUNT(*) FROM tasks
                     WHERE tasks.run_id = stores.run_id
                       AND tasks.current_stage = stores.stage
                       AND tasks.state IN
                           ('queued','running','gated','revising','joining')
                )
              WHERE run_id = ?",
        )
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;
    use std::sync::Arc;

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

    #[tokio::test]
    async fn ensure_is_idempotent_and_starts_empty() {
        let repo = StoreRepo::new(fresh_pool().await);
        repo.ensure("R1", "spec", 3).await.unwrap();
        // re-ensure does not reset / duplicate
        repo.ensure("R1", "spec", 99).await.unwrap();
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(0));
        // capacity is the original (re-ensure ignored)
        assert!(!repo.is_full("R1", "spec").await.unwrap());
    }

    #[tokio::test]
    async fn reserve_respects_capacity_then_backpressures() {
        let repo = StoreRepo::new(fresh_pool().await);
        repo.ensure("R1", "spec", 2).await.unwrap();
        assert!(repo.reserve("R1", "spec").await.unwrap(), "slot 1");
        assert!(repo.reserve("R1", "spec").await.unwrap(), "slot 2");
        assert!(!repo.reserve("R1", "spec").await.unwrap(), "full -> backpressure");
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(2));
        assert!(repo.is_full("R1", "spec").await.unwrap());
    }

    #[tokio::test]
    async fn release_frees_a_slot() {
        let repo = StoreRepo::new(fresh_pool().await);
        repo.ensure("R1", "spec", 1).await.unwrap();
        assert!(repo.reserve("R1", "spec").await.unwrap());
        assert!(!repo.reserve("R1", "spec").await.unwrap(), "full");
        assert!(repo.release("R1", "spec").await.unwrap(), "freed");
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(0));
        assert!(repo.reserve("R1", "spec").await.unwrap(), "slot reusable");
    }

    #[tokio::test]
    async fn release_never_goes_negative() {
        let repo = StoreRepo::new(fresh_pool().await);
        repo.ensure("R1", "spec", 1).await.unwrap();
        assert!(!repo.release("R1", "spec").await.unwrap(), "empty release is a no-op");
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(0));
    }

    #[tokio::test]
    async fn missing_store_reserve_and_queries() {
        let repo = StoreRepo::new(fresh_pool().await);
        assert!(!repo.reserve("R1", "ghost").await.unwrap(), "no store -> no slot");
        assert_eq!(repo.occupancy("R1", "ghost").await.unwrap(), None);
        assert!(!repo.is_full("R1", "ghost").await.unwrap());
    }

    #[tokio::test]
    async fn concurrent_over_reserve_yields_exactly_capacity_winners() {
        // THE CRUX: spawn N concurrent reservers against a capacity-C store; the
        // `occupancy < capacity` guard means exactly C win and occupancy lands at
        // C — never above.
        const CAP: u32 = 5;
        const N: u32 = 50;
        let repo = Arc::new(StoreRepo::new(fresh_pool().await));
        repo.ensure("R1", "spec", CAP).await.unwrap();

        let mut handles = Vec::new();
        for _ in 0..N {
            let r = repo.clone();
            handles.push(tokio::spawn(async move { r.reserve("R1", "spec").await.unwrap() }));
        }
        let mut winners = 0u32;
        for h in handles {
            if h.await.unwrap() {
                winners += 1;
            }
        }
        assert_eq!(winners, CAP, "exactly capacity reservers win");
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(CAP));
    }

    async fn insert_task(pool: &SqlitePool, id: &str, run_id: &str, stage: &str, state: &str) {
        sqlx::query(
            "INSERT INTO tasks
               (id, project_id, pipeline, topic, current_stage, state, attempts,
                created_at, updated_at, run_id)
             VALUES (?,?,?,?,?,?,0,100,100,?)",
        )
        .bind(id)
        .bind("p")
        .bind("pipe")
        .bind("topic")
        .bind(stage)
        .bind(state)
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn reconcile_sets_occupancy_to_resident_count() {
        let pool = fresh_pool().await;
        let repo = StoreRepo::new(pool.clone());
        repo.ensure("R1", "spec", 5).await.unwrap();
        // Inflate occupancy as a kill/crash would leave it.
        repo.reserve("R1", "spec").await.unwrap();
        repo.reserve("R1", "spec").await.unwrap();
        repo.reserve("R1", "spec").await.unwrap();
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(3));

        // Reality: 2 resident items at `spec` (queued + running), 1 done (left),
        // 1 needs_human (left), and one item at a different stage.
        insert_task(&pool, "t-queued", "R1", "spec", "queued").await;
        insert_task(&pool, "t-running", "R1", "spec", "running").await;
        insert_task(&pool, "t-done", "R1", "spec", "done").await;
        insert_task(&pool, "t-nh", "R1", "spec", "needs_human").await;
        insert_task(&pool, "t-other", "R1", "plan", "queued").await;

        repo.reconcile_occupancy("R1").await.unwrap();
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn reconcile_clears_a_fully_leaked_reservation_to_zero() {
        let pool = fresh_pool().await;
        let repo = StoreRepo::new(pool.clone());
        repo.ensure("R1", "spec", 3).await.unwrap();
        repo.reserve("R1", "spec").await.unwrap();
        repo.reserve("R1", "spec").await.unwrap();
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(2));
        // No resident tasks at all -> occupancy must drop to 0.
        repo.reconcile_occupancy("R1").await.unwrap();
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(0));
    }

    #[tokio::test]
    async fn reconcile_counts_gated_revising_joining_as_resident() {
        let pool = fresh_pool().await;
        let repo = StoreRepo::new(pool.clone());
        repo.ensure("R1", "gate-1", 9).await.unwrap();
        insert_task(&pool, "g1", "R1", "gate-1", "gated").await;
        insert_task(&pool, "g2", "R1", "gate-1", "revising").await;
        insert_task(&pool, "g3", "R1", "gate-1", "joining").await;
        repo.reconcile_occupancy("R1").await.unwrap();
        assert_eq!(repo.occupancy("R1", "gate-1").await.unwrap(), Some(3));
    }

    #[tokio::test]
    async fn reconcile_only_touches_the_given_run() {
        let pool = fresh_pool().await;
        let repo = StoreRepo::new(pool.clone());
        repo.ensure("R1", "spec", 5).await.unwrap();
        repo.ensure("R2", "spec", 5).await.unwrap();
        repo.reserve("R2", "spec").await.unwrap(); // R2 leaked
        repo.reserve("R2", "spec").await.unwrap();
        insert_task(&pool, "a", "R1", "spec", "queued").await;
        repo.reconcile_occupancy("R1").await.unwrap();
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(1));
        // R2 untouched by an R1 reconcile.
        assert_eq!(repo.occupancy("R2", "spec").await.unwrap(), Some(2));
    }
}
