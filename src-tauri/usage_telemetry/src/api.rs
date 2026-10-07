//! Usage Telemetry OHS — the context's Tauri commands + the `tools()` catalog
//! consumed by Conversational Control (Plan 6). State holds the two stores + the
//! config; `usage_snapshot` recomputes on demand. The brake STATE is Runtime's — these commands never touch it.

use crate::snapshot::{compute_snapshot, UsageConfig, UsageSnapshot};
use crate::utilization_store::UtilizationStore;
use crate::worker_log::WorkerUsageStore;
use agent_bus_core::ToolSpec;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-tool argument types for Usage Telemetry's OHS tools (T1). Each tool's
/// `input_schema` is DERIVED from these via `schemars` (one source of truth —
/// the dispatcher deserializes the same struct). Owned by the supplier; the
/// agentic loop validates only the published JSON schema, never these types.
pub mod args {
    use super::{Deserialize, JsonSchema};

    /// `usage_snapshot` — no arguments.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema, Default)]
    pub struct UsageSnapshotArgs {}

    /// `usage_set_auto_meter` — enable/disable the reactive auto-meter brake.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
    pub struct SetAutoMeterArgs {
        pub enabled: bool,
    }
}

/// Shared Telemetry state held by Tauri's state manager. `braked` is read from a
/// callback the root installs (so Telemetry doesn't depend on Runtime's type).
pub struct UsageState {
    pub worker: Arc<WorkerUsageStore>,
    pub util: Arc<UtilizationStore>,
    pub pool: SqlitePool,
    /// Returns Runtime's current brake on/off. Installed by the root (D9).
    pub is_braked: Arc<dyn Fn() -> bool + Send + Sync>,
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// Load the single config row (id=1). Falls back to defaults if absent.
pub async fn load_config(pool: &SqlitePool) -> UsageConfig {
    let row: Option<(i64, f64, f64, i64)> = sqlx::query_as(
        "SELECT window_secs, brake_on_pct, brake_off_pct, auto_meter_enabled
         FROM usage_config WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    match row {
        Some((secs, on, off, auto)) => UsageConfig {
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
    compute_snapshot(&state.worker, &state.util, &cfg, braked, now_unix())
        .await
        .map_err(|e| e.to_string())
}

/// Write the auto-meter enable flag to the config row (R2). Pure DB write —
/// the brake STATE stays Runtime's; the root's sweep reads this each tick.
pub async fn set_auto_meter_inner(pool: &SqlitePool, enabled: bool) -> Result<(), String> {
    sqlx::query("UPDATE usage_config SET auto_meter_enabled = ? WHERE id = 1")
        .bind(if enabled { 1 } else { 0 })
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn usage_set_auto_meter(
    state: tauri::State<'_, UsageState>,
    enabled: bool,
) -> Result<UsageSnapshot, String> {
    set_auto_meter_inner(&state.pool, enabled).await?;
    usage_snapshot(state).await
}

/// The JSON Schema for an arg struct, derived via `schemars` (T1 — one source of
/// truth with the typed dispatch path). PUBLIC so T2 (N-NativeToolUse) can reuse
/// it as the forced tool-use `input_schema` on the API path.
pub fn arg_schema<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_else(|_| json!({ "type": "object" }))
}

/// OHS contract — consumed by Conversational Control (Plan 6). Each
/// `input_schema` is DERIVED from its arg struct (T1 anti-drift).
pub fn tools() -> Vec<ToolSpec> {
    let ctx = "usage-telemetry";
    vec![
        ToolSpec {
            name: "usage_snapshot".into(),
            description: "Return the current plan usage meter: session and weekly Utilization with reset times, per-team tokens and list-price Cost.".into(),
            input_schema: arg_schema::<args::UsageSnapshotArgs>(),
            supplier_context: ctx.into(),
        },
        ToolSpec {
            name: "usage_set_auto_meter".into(),
            description: "Enable or disable the auto-brake (brakes new work at 95% of the session or weekly limit).".into(),
            input_schema: arg_schema::<args::SetAutoMeterArgs>(),
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
        assert_eq!(cfg, UsageConfig::default());
        assert_eq!(cfg.window_secs, 18_000);
        assert_eq!(cfg.brake_on_pct, 0.95);
        assert_eq!(cfg.brake_off_pct, 0.85);
        assert!(!cfg.auto_meter_enabled); // default OFF
    }

    #[tokio::test]
    async fn set_auto_meter_persists_to_config() {
        let pool = fresh_pool().await;
        assert!(!load_config(&pool).await.auto_meter_enabled);
        set_auto_meter_inner(&pool, true).await.unwrap();
        assert!(load_config(&pool).await.auto_meter_enabled);
        set_auto_meter_inner(&pool, false).await.unwrap();
        assert!(!load_config(&pool).await.auto_meter_enabled);
    }

    #[test]
    fn set_auto_meter_args_round_trip_and_require_bool() {
        let a: args::SetAutoMeterArgs = serde_json::from_value(json!({ "enabled": true })).unwrap();
        assert!(a.enabled);
        assert!(serde_json::from_value::<args::SetAutoMeterArgs>(json!({})).is_err());
        assert!(serde_json::from_value::<args::SetAutoMeterArgs>(json!({ "enabled": "yes" })).is_err());
    }

    #[test]
    fn tools_include_set_auto_meter() {
        let t = tools();
        assert!(t.iter().any(|s| s.name == "usage_set_auto_meter"));
        assert!(!t.iter().any(|s| s.name == "usage_set_budget"));
    }

    #[test]
    fn derived_schemas_carry_the_right_required_props() {
        let s = |n: &str| tools().into_iter().find(|t| t.name == n).unwrap().input_schema;
        // usage_set_auto_meter: enabled required + boolean.
        let m = s("usage_set_auto_meter");
        assert!(m["required"].as_array().unwrap().iter().any(|v| v == "enabled"));
        // usage_snapshot: no required props.
        let snap = s("usage_snapshot");
        assert_eq!(snap["required"].as_array().map(|a| a.len()).unwrap_or(0), 0);
        // GENERATED, not literal.
        assert_eq!(m, arg_schema::<args::SetAutoMeterArgs>());
    }

    #[test]
    fn tools_are_usage_telemetry_slug() {
        let t = tools();
        assert!(t.iter().all(|s| s.supplier_context == "usage-telemetry"));
        for name in ["usage_snapshot", "usage_set_auto_meter"] {
            assert!(t.iter().any(|s| s.name == name), "missing tool {name}");
        }
    }
}
