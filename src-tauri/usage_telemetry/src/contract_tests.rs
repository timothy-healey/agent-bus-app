//! Serde contract regression tests for the Usage Telemetry IPC return types
//! `UsageSnapshot`, `TeamSlice`, and the `ThresholdBand` enum. Lock the
//! serialized JSON key set + enum strings against the TS interfaces. The
//! snapshot is built FULLY populated (reset_in_secs / est_brake_at = Some) so
//! every key appears. Additive only.

#![cfg(test)]

use crate::snapshot::{TeamSlice, UsageSnapshot};
use crate::window::ThresholdBand;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("object").keys().cloned().collect()
}
fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn full_snapshot() -> UsageSnapshot {
    let mut tokens_by_task = HashMap::new();
    tokens_by_task.insert("T-1".to_string(), 120u64);
    UsageSnapshot {
        window_total: 2120,
        window_budget: 4240,
        window_pct: 0.5,
        band: ThresholdBand::Warn,
        burn_per_min: 12.5,
        window_secs: 18_000,
        reset_in_secs: Some(900),     // Some so the optional key appears
        est_brake_at: Some(1_700_000_000), // Some so the optional key appears
        by_team: vec![TeamSlice { team_id: "research".into(), tokens: 120 }],
        tokens_by_task,
        braked: false,
        auto_meter_enabled: false,
    }
}

/// Locks `src/ipc/usage.ts:10-22` `interface UsageSnapshot`:
/// { window_total, window_budget, window_pct, band, burn_per_min, window_secs,
///   reset_in_secs, est_brake_at, by_team, tokens_by_task, braked }.
#[test]
fn usage_snapshot_key_set_matches_ts() {
    let v = serde_json::to_value(full_snapshot()).unwrap();
    assert_eq!(
        keys(&v),
        set(&[
            "window_total",
            "window_budget",
            "window_pct",
            "band",
            "burn_per_min",
            "window_secs",
            "reset_in_secs",
            "est_brake_at",
            "by_team",
            "tokens_by_task",
            "braked",
            "auto_meter_enabled",
        ]),
    );
    assert_eq!(v["band"], Value::String("warn".into()));
    assert!(v["by_team"].is_array());
    // tokens_by_task is TS Record<string, number> → JSON object.
    assert!(v["tokens_by_task"].is_object());
    assert_eq!(v["tokens_by_task"]["T-1"], serde_json::json!(120));
    assert!(v["braked"].is_boolean());
}

/// `reset_in_secs` / `est_brake_at` are `number | null` in TS: present as `null`
/// keys when None. Locks `src/ipc/usage.ts:17-18`.
#[test]
fn usage_snapshot_none_options_are_present_and_null() {
    let mut s = full_snapshot();
    s.reset_in_secs = None;
    s.est_brake_at = None;
    let v = serde_json::to_value(&s).unwrap();
    assert!(v.as_object().unwrap().contains_key("reset_in_secs"));
    assert!(v["reset_in_secs"].is_null());
    assert!(v["est_brake_at"].is_null());
}

/// Locks `src/ipc/usage.ts:5-8` `interface TeamSlice { team_id; tokens }`.
#[test]
fn team_slice_key_set_matches_ts() {
    let v = serde_json::to_value(TeamSlice { team_id: "research".into(), tokens: 120 }).unwrap();
    assert_eq!(keys(&v), set(&["team_id", "tokens"]));
    assert!(v["tokens"].is_number());
}

/// Locks `src/ipc/usage.ts:3` `type ThresholdBand = "safe" | "warn" | "hot" | "braked"`.
#[test]
fn threshold_band_matches_ts_string_union() {
    let cases = [
        (ThresholdBand::Safe, "safe"),
        (ThresholdBand::Warn, "warn"),
        (ThresholdBand::Hot, "hot"),
        (ThresholdBand::Braked, "braked"),
    ];
    for (variant, s) in cases {
        assert_eq!(serde_json::to_value(variant).unwrap(), Value::String(s.into()));
    }
}
