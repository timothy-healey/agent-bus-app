//! UsageSnapshot — the single value the meter renders. Built from the latest
//! Utilization reading (the meter's headline + band) and the worker log
//! (per-team tokens and Cost in the 5-hour window, per-task tokens).

use crate::utilization_store::UtilizationStore;
use crate::window::{band, ThresholdBand};
use crate::brake_policy::watched_fraction;
use crate::worker_log::{WorkerUsageError, WorkerUsageStore};
use agent_bus_core::{LimitKind, LimitReading};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error(transparent)]
    Worker(#[from] WorkerUsageError),
}

/// Meter config, loaded from `usage_config` (row id=1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsageConfig {
    /// The per-team Cost window (the session Limit's length).
    pub window_secs: i64,
    pub brake_on_pct: f64,
    pub brake_off_pct: f64,
    pub auto_meter_enabled: bool,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self { window_secs: 18_000, brake_on_pct: 0.95, brake_off_pct: 0.85, auto_meter_enabled: false }
    }
}

/// One Limit as the meter shows it. An expired Limit reads 0%.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitView {
    pub label: String,
    pub utilization_pct: f64,
    pub resets_in_secs: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamSlice {
    pub team_id: String,
    pub tokens: u64,
    /// List-price Cost in the window. Informational only.
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    /// The last poll succeeded.
    pub available: bool,
    /// When the last good reading was taken (unix seconds).
    pub observed_at: Option<i64>,
    pub session: Option<LimitView>,
    pub weekly: Option<LimitView>,
    pub model_scoped: Vec<LimitView>,
    pub band: ThresholdBand,
    pub braked: bool,
    pub auto_meter_enabled: bool,
    pub by_team: Vec<TeamSlice>,
    /// task_id -> lifetime tokens, for the board cards.
    pub tokens_by_task: HashMap<String, u64>,
}

fn view(l: &LimitReading, now: i64) -> LimitView {
    let label = match &l.kind {
        LimitKind::Session => "session (5h)".to_string(),
        LimitKind::Weekly => "weekly (7d)".to_string(),
        LimitKind::ModelWeekly { model } => format!("{model} weekly"),
    };
    let expired = now >= l.resets_at;
    LimitView {
        label,
        utilization_pct: if expired { 0.0 } else { l.utilization_pct },
        resets_in_secs: (l.resets_at - now).max(0),
    }
}

/// Assemble a snapshot. `braked` is Runtime's brake state; `now` is injected.
pub async fn compute_snapshot(
    worker: &WorkerUsageStore,
    util: &UtilizationStore,
    cfg: &UsageConfig,
    braked: bool,
    now: i64,
) -> Result<UsageSnapshot, SnapshotError> {
    let stored = util.load().await;
    let (mut session, mut weekly, mut model_scoped) = (None, None, Vec::new());
    if let Some(r) = &stored.reading {
        for l in &r.limits {
            match l.kind {
                LimitKind::Session => session = Some(view(l, now)),
                LimitKind::Weekly => weekly = Some(view(l, now)),
                LimitKind::ModelWeekly { .. } => model_scoped.push(view(l, now)),
            }
        }
    }
    let fraction = stored.reading.as_ref().and_then(|r| watched_fraction(r, now)).unwrap_or(0.0);
    let by_team = worker
        .team_breakdown(now - cfg.window_secs)
        .await?
        .into_iter()
        .map(|t| TeamSlice { team_id: t.team_id, tokens: t.tokens, cost_usd: t.cost_micros as f64 / 1_000_000.0 })
        .collect();
    let tokens_by_task = worker.tokens_by_task().await?.into_iter().collect();
    Ok(UsageSnapshot {
        available: stored.available(),
        observed_at: stored.reading.as_ref().map(|r| r.observed_at),
        session,
        weekly,
        model_scoped,
        band: band(fraction, braked),
        braked,
        auto_meter_enabled: cfg.auto_meter_enabled,
        by_team,
        tokens_by_task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utilization_store::UtilizationStore;
    use crate::worker_log::WorkerUsageStore;
    use agent_bus_core::{LimitKind, LimitReading, TaskId, TeamId, UsageEvent, UtilizationReading};
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/005_usage.sql")).execute(&pool).await.unwrap();
        sqlx::query("ALTER TABLE worker_usage_log ADD COLUMN cost_micros INTEGER").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE utilization_state (id INTEGER PRIMARY KEY CHECK (id = 1), reading_json TEXT, last_ok_at INTEGER, last_attempt_at INTEGER, last_error TEXT)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO utilization_state (id) VALUES (1)").execute(&pool).await.unwrap();
        pool
    }

    fn reading(session: f64, weekly: f64) -> UtilizationReading {
        UtilizationReading { observed_at: 900, limits: vec![
            LimitReading { kind: LimitKind::Session, utilization_pct: session, resets_at: 1_000 + 3_600 },
            LimitReading { kind: LimitKind::Weekly, utilization_pct: weekly, resets_at: 1_000 + 86_400 },
            LimitReading { kind: LimitKind::ModelWeekly { model: "Fable".into() }, utilization_pct: 0.0, resets_at: 1_000 + 86_400 },
        ]}
    }

    fn ev(ts: i64, team: &str, cost_micros: Option<u64>) -> UsageEvent {
        UsageEvent { ts, team_id: TeamId(team.into()), task_id: Some(TaskId("T-1".into())), model: "m".into(),
            input_tokens: 100, output_tokens: 20, cache_creation: 0, cache_read: 0, cost_micros }
    }

    #[tokio::test]
    async fn never_polled_is_unavailable_with_no_limits() {
        let p = pool().await;
        let snap = compute_snapshot(&WorkerUsageStore::new(p.clone()), &UtilizationStore::new(p), &UsageConfig::default(), false, 1_000).await.unwrap();
        assert!(!snap.available);
        assert!(snap.session.is_none() && snap.weekly.is_none());
        assert_eq!(snap.band, ThresholdBand::Safe);
    }

    #[tokio::test]
    async fn band_comes_from_the_higher_watched_limit() {
        let p = pool().await;
        let util = UtilizationStore::new(p.clone());
        util.record_ok(&reading(41.0, 88.0), 900).await;
        let snap = compute_snapshot(&WorkerUsageStore::new(p), &util, &UsageConfig::default(), false, 1_000).await.unwrap();
        assert!(snap.available);
        assert_eq!(snap.observed_at, Some(900));
        assert_eq!(snap.session.as_ref().unwrap().utilization_pct, 41.0);
        assert_eq!(snap.session.as_ref().unwrap().resets_in_secs, 3_600);
        assert_eq!(snap.weekly.as_ref().unwrap().label, "weekly (7d)");
        assert_eq!(snap.model_scoped.len(), 1);
        assert_eq!(snap.model_scoped[0].label, "Fable weekly");
        assert_eq!(snap.band, ThresholdBand::Hot);
    }

    #[tokio::test]
    async fn a_failed_poll_keeps_the_last_reading_but_reports_unavailable() {
        let p = pool().await;
        let util = UtilizationStore::new(p.clone());
        util.record_ok(&reading(41.0, 2.0), 900).await;
        util.record_err("usage query timed out", 950).await;
        let snap = compute_snapshot(&WorkerUsageStore::new(p), &util, &UsageConfig::default(), false, 1_000).await.unwrap();
        assert!(!snap.available);
        assert_eq!(snap.session.unwrap().utilization_pct, 41.0);
    }

    #[tokio::test]
    async fn by_team_sums_cost_in_the_window_and_tolerates_null_cost() {
        let p = pool().await;
        let worker = WorkerUsageStore::new(p.clone());
        worker.insert(&ev(900, "research", Some(250_000))).await.unwrap();
        worker.insert(&ev(950, "research", None)).await.unwrap();
        worker.insert(&ev(10, "research", Some(9_000_000))).await.unwrap(); // outside the 5h window ending at 20_000
        let cfg = UsageConfig::default();
        let snap = compute_snapshot(&worker, &UtilizationStore::new(p), &cfg, false, 18_500).await.unwrap();
        let research = snap.by_team.iter().find(|t| t.team_id == "research").unwrap();
        assert_eq!(research.cost_usd, 0.25);
        assert_eq!(research.tokens, 240);
    }
}
