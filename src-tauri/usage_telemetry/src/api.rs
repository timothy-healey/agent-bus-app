//! Usage Telemetry OHS — the context's Tauri commands + the `tools()` catalog
//! consumed by Conversational Control (Plan 6). State holds the two stores + the
//! config; `usage_snapshot` recomputes on demand, `usage_set_budget` writes the
//! config row. The brake STATE is Runtime's — these commands never touch it.

use crate::cc_log::CcUsageStore;
use crate::snapshot::{compute_snapshot, UsageConfig, UsageSnapshot};
use crate::worker_log::WorkerUsageStore;
use agent_bus_core::ToolSpec;
use serde_json::json;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Shared Telemetry state held by Tauri's state manager. `braked` is read from a
/// callback the root installs (so Telemetry doesn't depend on Runtime's type).
pub struct UsageState {
    pub cc: Arc<CcUsageStore>,
    pub worker: Arc<WorkerUsageStore>,
    pub pool: SqlitePool,
    /// Returns Runtime's current brake on/off. Installed by the root (D9).
    pub is_braked: Arc<dyn Fn() -> bool + Send + Sync>,
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// Load the single config row (id=1). Falls back to defaults if absent.
pub async fn load_config(pool: &SqlitePool) -> UsageConfig {
    let row: Option<(i64, i64, f64, f64, i64)> = sqlx::query_as(
        "SELECT window_budget, window_secs, brake_on_pct, brake_off_pct, auto_meter_enabled
         FROM usage_config WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    match row {
        Some((budget, secs, on, off, auto)) => UsageConfig {
            window_budget: budget as u64,
            window_secs: secs,
            brake_on_pct: on,
            brake_off_pct: off,
            auto_meter_enabled: auto != 0,
        },
        None => UsageConfig::default(),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn usage_snapshot(state: tauri::State<'_, UsageState>) -> Result<UsageSnapshot, String> {
    let cfg = load_config(&state.pool).await;
    let braked = (state.is_braked)();
    compute_snapshot(&state.cc, &state.worker, &cfg, braked, now_unix())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn usage_set_budget(
    state: tauri::State<'_, UsageState>,
    budget: i64,
) -> Result<UsageSnapshot, String> {
    if budget <= 0 {
        return Err("budget must be positive".into());
    }
    sqlx::query("UPDATE usage_config SET window_budget = ? WHERE id = 1")
        .bind(budget)
        .execute(&state.pool)
        .await
        .map_err(|e| e.to_string())?;
    usage_snapshot(state).await
}

/// OHS contract — consumed by Conversational Control (Plan 6).
pub fn tools() -> Vec<ToolSpec> {
    let ctx = "usage-telemetry";
    vec![
        ToolSpec {
            name: "usage_snapshot".into(),
            description: "Return the current rolling-window usage meter (total, %, band, burn rate, per-team breakdown).".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "usage_set_budget".into(),
            description: "Set the rolling-window token budget (the meter's denominator).".into(),
            input_schema: json!({ "type": "object", "properties": { "budget": { "type": "integer" } }, "required": ["budget"] }),
            supplier_context: ctx.into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/005_usage.sql")).execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn load_config_returns_seeded_defaults() {
        let cfg = load_config(&fresh_pool().await).await;
        assert_eq!(cfg.window_budget, 2_600_000);
        assert_eq!(cfg.window_secs, 18_000);
        assert!(!cfg.auto_meter_enabled); // v1 default OFF (D8)
    }

    #[tokio::test]
    async fn updating_budget_changes_loaded_config() {
        let pool = fresh_pool().await;
        sqlx::query("UPDATE usage_config SET window_budget = ? WHERE id = 1")
            .bind(5_000_000i64)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(load_config(&pool).await.window_budget, 5_000_000);
    }

    #[test]
    fn tools_are_usage_telemetry_slug() {
        let t = tools();
        assert!(t.iter().all(|s| s.supplier_context == "usage-telemetry"));
        for name in ["usage_snapshot", "usage_set_budget"] {
            assert!(t.iter().any(|s| s.name == name), "missing tool {name}");
        }
    }
}
