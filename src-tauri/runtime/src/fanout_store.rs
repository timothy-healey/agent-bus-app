//! FanOutStore — SQLite persistence + the barrier for the FanOutGroup aggregate.
//! The completes-exactly-once invariant is enforced by the same conditional-
//! UPDATE single-writer pattern claim_next_for_stage uses: `UPDATE fanout_groups
//! SET completed=1 WHERE id=? AND completed=0` — rows_affected == 1 is the sole
//! winner that creates the continuation (vet F1).

use crate::fanout_group::{Continuation, FanOutGroup, LaneVerdict};
use agent_bus_core::Verdict;
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FanOutStoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("group not found: {0}")]
    NotFound(String),
    #[error("corrupt verdict string in db: {0}")]
    BadVerdict(String),
}

/// The result of recording a lane verdict at the barrier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarrierOutcome {
    /// Recorded; the group is not complete by this caller (lanes still
    /// outstanding, or another settlement already won). This lane parks.
    Parked,
    /// This caller is the sole winner of the completes-once guard and must
    /// create the single continuation.
    Completed(Continuation),
}

fn verdict_str(v: Verdict) -> &'static str {
    match v {
        Verdict::Approve => "approve",
        Verdict::Revise => "revise",
        Verdict::Reject => "reject",
    }
}

fn parse_verdict(s: &str) -> Option<Verdict> {
    Some(match s {
        "approve" => Verdict::Approve,
        "revise" => Verdict::Revise,
        "reject" => Verdict::Reject,
        _ => return None,
    })
}

pub struct FanOutStore {
    pool: SqlitePool,
}

impl FanOutStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Persist a new fan-out group (one row per fork expansion).
    pub async fn create(&self, g: &FanOutGroup) -> Result<(), FanOutStoreError> {
        sqlx::query(
            "INSERT INTO fanout_groups (id, pipeline, join_target, downstream, completed, created_at)
             VALUES (?,?,?,?,0,0)",
        )
        .bind(&g.id)
        .bind(&g.pipeline)
        .bind(&g.join_target)
        .bind(&g.downstream)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load a group + its expected lanes (seeded at create time via `seed_lane`).
    pub async fn load(&self, group_id: &str) -> Result<FanOutGroup, FanOutStoreError> {
        let row = sqlx::query_as::<_, (String, String, String, i64)>(
            "SELECT pipeline, join_target, downstream, completed FROM fanout_groups WHERE id = ?",
        )
        .bind(group_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| FanOutStoreError::NotFound(group_id.to_string()))?;
        let lanes: Vec<(String,)> =
            sqlx::query_as("SELECT lane FROM fanout_lanes WHERE group_id = ? ORDER BY lane")
                .bind(group_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(FanOutGroup {
            id: group_id.to_string(),
            pipeline: row.0,
            join_target: row.1,
            downstream: row.2,
            expected_lanes: lanes.into_iter().map(|(l,)| l).collect(),
            completed: row.3 != 0,
        })
    }

    /// Seed the expected lane set with a pending placeholder so `load` knows the
    /// full lane set before any verdict is recorded. Called by the pool right
    /// after `create`, once per lane. A pending row has verdict='pending' and is
    /// treated as not-yet-settled by the barrier.
    pub async fn seed_lane(&self, group_id: &str, lane: &str) -> Result<(), FanOutStoreError> {
        sqlx::query(
            "INSERT INTO fanout_lanes (group_id, lane, verdict) VALUES (?,?,'pending')
             ON CONFLICT(group_id, lane) DO NOTHING",
        )
        .bind(group_id)
        .bind(lane)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record this lane's settled verdict, then attempt to complete the group
    /// exactly once. The single-row conditional UPDATE is the guard (vet F1).
    pub async fn record_and_try_complete(
        &self,
        group_id: &str,
        lane: &str,
        verdict: Verdict,
    ) -> Result<BarrierOutcome, FanOutStoreError> {
        // 1. Upsert this lane's verdict (idempotent on re-record — D6).
        sqlx::query(
            "INSERT INTO fanout_lanes (group_id, lane, verdict) VALUES (?,?,?)
             ON CONFLICT(group_id, lane) DO UPDATE SET verdict=excluded.verdict",
        )
        .bind(group_id)
        .bind(lane)
        .bind(verdict_str(verdict))
        .execute(&self.pool)
        .await?;

        // 2. Read all lane verdicts; bail if any lane is still pending.
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT lane, verdict FROM fanout_lanes WHERE group_id = ?")
                .bind(group_id)
                .fetch_all(&self.pool)
                .await?;
        let any_pending = rows.iter().any(|(_, v)| v == "pending");
        if rows.is_empty() || any_pending {
            return Ok(BarrierOutcome::Parked);
        }

        // 3. All lanes settled — attempt the completes-once guard.
        let res = sqlx::query("UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0")
            .bind(group_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            // Another settlement already won, or already completed (idempotent).
            return Ok(BarrierOutcome::Parked);
        }

        // 4. This caller is the sole winner — compute the continuation.
        let g = self.load(group_id).await?;
        let recorded: Vec<LaneVerdict> = rows
            .into_iter()
            .map(|(lane, v)| {
                parse_verdict(&v)
                    .map(|verdict| LaneVerdict { lane, verdict })
                    .ok_or_else(|| FanOutStoreError::BadVerdict(v.clone()))
            })
            .collect::<Result<_, _>>()?;
        Ok(BarrierOutcome::Completed(g.continuation(&recorded)))
    }

    /// Early-cancel path (P2): record this failing lane's verdict, then complete
    /// the group to needs-human IMMEDIATELY — without waiting for the other lanes
    /// — using the SAME completes-once conditional UPDATE guard as the full
    /// barrier. This is the FanOutGroup aggregate's early-cancel policy; the
    /// caller invokes it only when the lane's join has `cancel_on_reject` and the
    /// lane settled as a failure (Verdict::Reject — covers explicit reject and
    /// revise-cap escalation per Decision D5).
    ///
    /// Returns `Completed(NeedsHuman)` for the sole winner; `Parked` if the group
    /// was already completed (a straggler / a concurrent settle won first). The
    /// exactly-one-completion invariant is preserved: the same `WHERE completed=0`
    /// row guard arbitrates between this path and `record_and_try_complete`.
    pub async fn record_failure_and_early_cancel(
        &self,
        group_id: &str,
        lane: &str,
        verdict: Verdict,
    ) -> Result<BarrierOutcome, FanOutStoreError> {
        // 1. Upsert this lane's verdict (idempotent on re-record — D6).
        sqlx::query(
            "INSERT INTO fanout_lanes (group_id, lane, verdict) VALUES (?,?,?)
             ON CONFLICT(group_id, lane) DO UPDATE SET verdict=excluded.verdict",
        )
        .bind(group_id)
        .bind(lane)
        .bind(verdict_str(verdict))
        .execute(&self.pool)
        .await?;

        // 2. Attempt the completes-once guard NOW — do not wait for other lanes.
        let res = sqlx::query("UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0")
            .bind(group_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            // Already completed (full barrier or another early-cancel won).
            return Ok(BarrierOutcome::Parked);
        }

        // 3. Sole winner: ask the aggregate root for the early-cancel
        //    continuation (keeps the aggregation rule on FanOutGroup — vet F1).
        let g = self.load(group_id).await?;
        Ok(BarrierOutcome::Completed(g.early_cancel_continuation()))
    }

    /// Quorum path (P3): record this lane's verdict, then ask the aggregate
    /// whether the quorum is decided. The aggregate resolves to Downstream once
    /// `quorum` lanes approve (early, without waiting for the rest) or to
    /// needs-human once that becomes impossible; until then it returns None and
    /// this lane parks. When decided, the SAME completes-once conditional UPDATE
    /// guard the full barrier uses arbitrates the single winner — exactly-once is
    /// preserved against concurrent settles and stragglers.
    pub async fn record_and_try_quorum(
        &self,
        group_id: &str,
        lane: &str,
        verdict: Verdict,
        quorum: u32,
    ) -> Result<BarrierOutcome, FanOutStoreError> {
        // 1. Upsert this lane's verdict (idempotent on re-record — D6).
        sqlx::query(
            "INSERT INTO fanout_lanes (group_id, lane, verdict) VALUES (?,?,?)
             ON CONFLICT(group_id, lane) DO UPDATE SET verdict=excluded.verdict",
        )
        .bind(group_id)
        .bind(lane)
        .bind(verdict_str(verdict))
        .execute(&self.pool)
        .await?;

        // 2. Read settled (non-pending) lane verdicts + ask the aggregate.
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT lane, verdict FROM fanout_lanes WHERE group_id = ? AND verdict != 'pending'")
                .bind(group_id)
                .fetch_all(&self.pool)
                .await?;
        let recorded: Vec<LaneVerdict> = rows
            .into_iter()
            .map(|(lane, v)| {
                parse_verdict(&v)
                    .map(|verdict| LaneVerdict { lane, verdict })
                    .ok_or_else(|| FanOutStoreError::BadVerdict(v.clone()))
            })
            .collect::<Result<_, _>>()?;
        let g = self.load(group_id).await?;
        let decided = match g.quorum_continuation(&recorded, quorum) {
            Some(c) => c,
            None => return Ok(BarrierOutcome::Parked),
        };

        // 3. Quorum decided — attempt the completes-once guard.
        let res = sqlx::query("UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0")
            .bind(group_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Ok(BarrierOutcome::Parked);
        }
        Ok(BarrierOutcome::Completed(decided))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fanout_group::{Continuation, FanOutGroup};
    use agent_bus_core::Verdict;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../../app/migrations/006_fanout.sql")).execute(&pool).await.unwrap();
        pool
    }

    fn group() -> FanOutGroup {
        FanOutGroup {
            id: "G-1".into(),
            pipeline: "pipe".into(),
            join_target: "join-1".into(),
            downstream: "after".into(),
            expected_lanes: vec!["lane-a".into(), "lane-b".into()],
            completed: false,
        }
    }

    async fn seeded(store: &FanOutStore) {
        store.create(&group()).await.unwrap();
        store.seed_lane("G-1", "lane-a").await.unwrap();
        store.seed_lane("G-1", "lane-b").await.unwrap();
    }

    #[tokio::test]
    async fn first_lane_records_and_does_not_complete() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded(&store).await;
        let outcome = store.record_and_try_complete("G-1", "lane-a", Verdict::Approve).await.unwrap();
        assert_eq!(outcome, BarrierOutcome::Parked);
    }

    #[tokio::test]
    async fn last_lane_completes_once_with_all_approve_downstream() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded(&store).await;
        store.record_and_try_complete("G-1", "lane-a", Verdict::Approve).await.unwrap();
        let outcome = store.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap();
        assert_eq!(outcome, BarrierOutcome::Completed(Continuation::Downstream("after".into())));
    }

    #[tokio::test]
    async fn one_reject_completes_to_needs_human() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded(&store).await;
        store.record_and_try_complete("G-1", "lane-a", Verdict::Approve).await.unwrap();
        let outcome = store.record_and_try_complete("G-1", "lane-b", Verdict::Reject).await.unwrap();
        assert_eq!(outcome, BarrierOutcome::Completed(Continuation::NeedsHuman));
    }

    #[tokio::test]
    async fn concurrent_double_settle_yields_exactly_one_completion() {
        // THE CRUX: two callers both observe "all lanes settled" and both attempt
        // completion. The WHERE completed=0 guard means exactly one wins.
        let store = std::sync::Arc::new(FanOutStore::new(fresh_pool().await));
        seeded(&store).await;
        store.record_and_try_complete("G-1", "lane-a", Verdict::Approve).await.unwrap();
        let s1 = store.clone();
        let s2 = store.clone();
        let h1 = tokio::spawn(async move { s1.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap() });
        let h2 = tokio::spawn(async move { s2.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap() });
        let (a, b) = (h1.await.unwrap(), h2.await.unwrap());
        let completions = [&a, &b].iter().filter(|o| matches!(o, BarrierOutcome::Completed(_))).count();
        assert_eq!(completions, 1, "exactly one caller completes the barrier");
    }

    #[tokio::test]
    async fn early_cancel_completes_to_needs_human_without_waiting_for_other_lanes() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded(&store).await; // lanes a + b, both 'pending'
        // lane-a fails; with early-cancel the group completes NOW even though
        // lane-b is still pending.
        let outcome = store
            .record_failure_and_early_cancel("G-1", "lane-a", Verdict::Reject)
            .await
            .unwrap();
        assert_eq!(outcome, BarrierOutcome::Completed(Continuation::NeedsHuman));
        // group is marked completed
        assert!(store.load("G-1").await.unwrap().completed);
    }

    #[tokio::test]
    async fn early_cancel_is_exactly_once_against_a_concurrent_straggler() {
        // THE CRUX for P2: the failing lane early-cancels while the other lane
        // settles concurrently. Exactly one of them wins the completes-once guard.
        let store = std::sync::Arc::new(FanOutStore::new(fresh_pool().await));
        seeded(&store).await;
        let s1 = store.clone();
        let s2 = store.clone();
        let h1 = tokio::spawn(async move {
            s1.record_failure_and_early_cancel("G-1", "lane-a", Verdict::Reject).await.unwrap()
        });
        let h2 = tokio::spawn(async move {
            s2.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap()
        });
        let (a, b) = (h1.await.unwrap(), h2.await.unwrap());
        let completions = [&a, &b].iter().filter(|o| matches!(o, BarrierOutcome::Completed(_))).count();
        assert_eq!(completions, 1, "exactly one caller completes the barrier");
        // and if the early-cancel won, it completed to needs-human
        if let BarrierOutcome::Completed(c) = &a {
            assert_eq!(*c, Continuation::NeedsHuman);
        }
    }

    #[tokio::test]
    async fn early_cancel_after_group_already_completed_parks() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded(&store).await;
        // first failing lane early-cancels (completes)
        store.record_failure_and_early_cancel("G-1", "lane-a", Verdict::Reject).await.unwrap();
        // a straggler lane settles later -> group already completed -> parks
        let again = store.record_and_try_complete("G-1", "lane-b", Verdict::Reject).await.unwrap();
        assert_eq!(again, BarrierOutcome::Parked);
    }

    fn group3() -> FanOutGroup {
        FanOutGroup {
            id: "G-3".into(), pipeline: "pipe".into(), join_target: "join-1".into(),
            downstream: "after".into(),
            expected_lanes: vec!["lane-a".into(), "lane-b".into(), "lane-c".into()],
            completed: false,
        }
    }

    async fn seeded3(store: &FanOutStore) {
        store.create(&group3()).await.unwrap();
        for l in ["lane-a", "lane-b", "lane-c"] {
            store.seed_lane("G-3", l).await.unwrap();
        }
    }

    #[tokio::test]
    async fn quorum_reached_early_completes_to_downstream() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded3(&store).await; // 3 lanes
        let first = store.record_and_try_quorum("G-3", "lane-a", Verdict::Approve, 2).await.unwrap();
        assert_eq!(first, BarrierOutcome::Parked); // 1 of 2
        let second = store.record_and_try_quorum("G-3", "lane-b", Verdict::Approve, 2).await.unwrap();
        assert_eq!(second, BarrierOutcome::Completed(Continuation::Downstream("after".into())));
        assert!(store.load("G-3").await.unwrap().completed);
    }

    #[tokio::test]
    async fn quorum_impossible_completes_to_needs_human() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded3(&store).await; // 3 lanes, quorum 2
        store.record_and_try_quorum("G-3", "lane-a", Verdict::Reject, 2).await.unwrap();
        // after second reject: 0 approvals, 1 unsettled < 2 => needs-human
        let out = store.record_and_try_quorum("G-3", "lane-b", Verdict::Reject, 2).await.unwrap();
        assert_eq!(out, BarrierOutcome::Completed(Continuation::NeedsHuman));
    }

    #[tokio::test]
    async fn quorum_straggler_after_completion_parks() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded3(&store).await;
        store.record_and_try_quorum("G-3", "lane-a", Verdict::Approve, 2).await.unwrap();
        store.record_and_try_quorum("G-3", "lane-b", Verdict::Approve, 2).await.unwrap(); // completes
        let straggler = store.record_and_try_quorum("G-3", "lane-c", Verdict::Approve, 2).await.unwrap();
        assert_eq!(straggler, BarrierOutcome::Parked);
    }

    #[tokio::test]
    async fn quorum_concurrent_settle_yields_exactly_one_completion() {
        let store = std::sync::Arc::new(FanOutStore::new(fresh_pool().await));
        seeded3(&store).await;
        store.record_and_try_quorum("G-3", "lane-a", Verdict::Approve, 2).await.unwrap();
        let s1 = store.clone();
        let s2 = store.clone();
        // two lanes approve concurrently; both observe quorum reached
        let h1 = tokio::spawn(async move { s1.record_and_try_quorum("G-3", "lane-b", Verdict::Approve, 2).await.unwrap() });
        let h2 = tokio::spawn(async move { s2.record_and_try_quorum("G-3", "lane-c", Verdict::Approve, 2).await.unwrap() });
        let (a, b) = (h1.await.unwrap(), h2.await.unwrap());
        let completions = [&a, &b].iter().filter(|o| matches!(o, BarrierOutcome::Completed(_))).count();
        assert_eq!(completions, 1, "exactly one caller completes the quorum barrier");
    }

    #[tokio::test]
    async fn re_recording_a_lane_is_idempotent() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded(&store).await;
        store.record_and_try_complete("G-1", "lane-a", Verdict::Approve).await.unwrap();
        store.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap();
        // Re-record lane-b after completion — must not complete a second time.
        let again = store.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap();
        assert_eq!(again, BarrierOutcome::Parked);
    }
}
