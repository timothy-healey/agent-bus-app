//! UsageSnapshot — the single value the meter renders + the brake policy reads.
//! `compute_snapshot` joins the two stores (D3: cc = window total, worker = team
//! breakdown + per-task cost) + window math + config into one serialisable view.
//! It is the only place the spec's window formula is realised end-to-end.

use crate::brake_policy::{decide, BrakeDecision};
use crate::cc_log::{CcUsageError, CcUsageStore};
use crate::window::{band, burn_per_min, est_brake_at, reset_in_secs, window_pct, ThresholdBand};
use crate::worker_log::{WorkerUsageError, WorkerUsageStore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error(transparent)]
    Cc(#[from] CcUsageError),
    #[error(transparent)]
    Worker(#[from] WorkerUsageError),
}

/// Meter config, loaded from `usage_config` (row id=1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsageConfig {
    pub window_budget: u64,
    pub window_secs: i64,
    pub brake_on_pct: f64,
    pub brake_off_pct: f64,
    pub auto_meter_enabled: bool,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            // all-tokens basis incl. cache (LF34): ~67.2M live throughput ≈ 35%
            // ⟹ ~192M; rounded. Tunable estimate, not an exact claude.ai mirror (G6).
            window_budget: 190_000_000,
            window_secs: 18_000,
            brake_on_pct: 0.95,
            brake_off_pct: 0.85,
            auto_meter_enabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamSlice {
    pub team_id: String,
    pub tokens: u64,
}

/// The full meter view model. Serialised over IPC to the topbar widget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub window_total: u64,
    pub window_budget: u64,
    pub window_pct: f64,
    pub band: ThresholdBand,
    pub burn_per_min: f64,
    pub window_secs: i64,
    /// Seconds until the window frees up (for the braked `↻` countdown). None
    /// when the window is empty.
    pub reset_in_secs: Option<i64>,
    /// Best-effort projected brake time (unix secs). None when burn is 0.
    pub est_brake_at: Option<i64>,
    pub by_team: Vec<TeamSlice>,
    /// task_id -> lifetime tokens, for the board cards (D12).
    pub tokens_by_task: HashMap<String, u64>,
    /// Whether the brake is currently on (passed in from Runtime by the root).
    pub braked: bool,
    /// Whether the reactive auto-meter brake is enabled (mirrors config; lets
    /// the UI render the Settings toggle + meter state). R2.
    pub auto_meter_enabled: bool,
}

/// Assemble a snapshot. `braked` is Runtime's current brake state (the root
/// passes `brake.is_on()`); `now` is injected (D5).
pub async fn compute_snapshot(
    cc: &CcUsageStore,
    worker: &WorkerUsageStore,
    cfg: &UsageConfig,
    braked: bool,
    now: i64,
) -> Result<UsageSnapshot, SnapshotError> {
    let since = now - cfg.window_secs;
    // D3: v1 window total = cc_usage_log; + API worker rows (zero in v1).
    let window_total = cc.window_tokens(since).await? + worker.api_window_tokens(since).await?;
    let pct = window_pct(window_total, cfg.window_budget);
    let recent = cc.recent_tokens(now, 60).await?;
    let burn = burn_per_min(recent, 60);
    let oldest = cc.oldest_in_window(since).await?;
    let by_team = worker
        .team_breakdown(since)
        .await?
        .into_iter()
        .map(|t| TeamSlice { team_id: t.team_id, tokens: t.tokens })
        .collect();
    let tokens_by_task = worker.tokens_by_task().await?.into_iter().collect();

    Ok(UsageSnapshot {
        window_total,
        window_budget: cfg.window_budget,
        window_pct: pct,
        band: band(pct, braked),
        burn_per_min: burn,
        window_secs: cfg.window_secs,
        reset_in_secs: reset_in_secs(oldest, cfg.window_secs, now),
        est_brake_at: if braked { None } else { est_brake_at(window_total, cfg.window_budget, cfg.brake_on_pct, burn, now) },
        by_team,
        tokens_by_task,
        braked,
        auto_meter_enabled: cfg.auto_meter_enabled,
    })
}

/// The auto-meter decision from a snapshot, honouring the config's enable flag
/// (D8). Returns NoChange when auto-meter is disabled (v1 default).
pub fn auto_brake_decision(snap: &UsageSnapshot, cfg: &UsageConfig, auto_on: bool) -> BrakeDecision {
    if !cfg.auto_meter_enabled {
        return BrakeDecision::NoChange;
    }
    decide(snap.window_pct, auto_on, cfg.brake_on_pct, cfg.brake_off_pct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::parse_transcript;
    use agent_bus_core::{TaskId, TeamId, UsageEvent};
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    const SAMPLE: &str = include_str!("fixtures/transcript-sample.jsonl");

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/005_usage.sql")).execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn snapshot_uses_cc_for_total_and_worker_for_breakdown() {
        let pool = fresh_pool().await;
        let cc = CcUsageStore::new(pool.clone());
        let worker = WorkerUsageStore::new(pool.clone());
        cc.ingest(&parse_transcript(SAMPLE)).await.unwrap(); // 2170 tokens (all-tokens incl. cache, LF34)

        // worker rows (team attribution + per-task cost); NOT counted in total (cli)
        let now = cc.oldest_in_window(0).await.unwrap().unwrap() + 100;
        worker.insert(&UsageEvent { ts: now, team_id: TeamId("research".into()), task_id: Some(TaskId("T-1".into())), model: "m".into(), input_tokens: 100, output_tokens: 20, cache_creation: 0, cache_read: 0 }).await.unwrap();

        let cfg = UsageConfig { window_budget: 4340, ..UsageConfig::default() };
        let snap = compute_snapshot(&cc, &worker, &cfg, false, now + 60).await.unwrap();
        assert_eq!(snap.window_total, 2170);          // cc only (D3); all tokens incl. cache (LF34)
        assert!((snap.window_pct - 0.5).abs() < 1e-9); // 2170 / 4340
        assert_eq!(snap.band, ThresholdBand::Safe);
        assert_eq!(snap.by_team, vec![TeamSlice { team_id: "research".into(), tokens: 120 }]);
        assert_eq!(snap.tokens_by_task.get("T-1"), Some(&120));
        assert!(!snap.braked);
    }

    #[tokio::test]
    async fn snapshot_reflects_auto_meter_enabled_flag() {
        let pool = fresh_pool().await;
        let cc = CcUsageStore::new(pool.clone());
        let worker = WorkerUsageStore::new(pool.clone());
        let cfg = UsageConfig { auto_meter_enabled: true, ..UsageConfig::default() };
        let snap = compute_snapshot(&cc, &worker, &cfg, false, 1000).await.unwrap();
        assert!(snap.auto_meter_enabled);
        let off = UsageConfig { auto_meter_enabled: false, ..UsageConfig::default() };
        let snap_off = compute_snapshot(&cc, &worker, &off, false, 1000).await.unwrap();
        assert!(!snap_off.auto_meter_enabled);
    }

    #[tokio::test]
    async fn auto_brake_decision_is_off_when_flag_disabled() {
        let pool = fresh_pool().await;
        let cc = CcUsageStore::new(pool.clone());
        let worker = WorkerUsageStore::new(pool.clone());
        let cfg = UsageConfig { auto_meter_enabled: false, ..UsageConfig::default() };
        let snap = compute_snapshot(&cc, &worker, &cfg, false, 1000).await.unwrap();
        // even at high pct the disabled flag forces NoChange
        let mut hot = snap.clone();
        hot.window_pct = 0.99;
        assert_eq!(auto_brake_decision(&hot, &cfg, false), BrakeDecision::NoChange);
    }

    #[tokio::test]
    async fn auto_brake_decision_trips_when_enabled_and_over() {
        let cfg = UsageConfig { auto_meter_enabled: true, ..UsageConfig::default() };
        let pool = fresh_pool().await;
        let snap = compute_snapshot(&CcUsageStore::new(pool.clone()), &WorkerUsageStore::new(pool.clone()), &cfg, false, 1000).await.unwrap();
        let mut hot = snap;
        hot.window_pct = 0.96;
        assert!(matches!(auto_brake_decision(&hot, &cfg, false), BrakeDecision::SetOn(_)));
    }
}
