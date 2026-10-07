//! Serde contract regression tests for the Usage Telemetry IPC types. Lock the
//! JSON key sets + enum strings against `src/ipc/usage.ts`.

#![cfg(test)]

use crate::snapshot::{LimitView, TeamSlice, UsageSnapshot};
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
    let lv = |label: &str| LimitView { label: label.into(), utilization_pct: 41.0, resets_in_secs: 600 };
    UsageSnapshot {
        available: true,
        observed_at: Some(1_700_000_000),
        session: Some(lv("session (5h)")),
        weekly: Some(lv("weekly (7d)")),
        model_scoped: vec![lv("Fable weekly")],
        band: ThresholdBand::Warn,
        braked: false,
        auto_meter_enabled: false,
        by_team: vec![TeamSlice { team_id: "research".into(), tokens: 120, cost_usd: 0.25 }],
        tokens_by_task,
        last_error: None,
    }
}

#[test]
fn usage_snapshot_key_set_matches_ts() {
    let v = serde_json::to_value(full_snapshot()).unwrap();
    assert_eq!(keys(&v), set(&[
        "available", "observed_at", "session", "weekly", "model_scoped", "band",
        "braked", "auto_meter_enabled", "by_team", "tokens_by_task", "last_error",
    ]));
    assert_eq!(v["band"], Value::String("warn".into()));
    assert!(v["tokens_by_task"].is_object());
}

#[test]
fn absent_limits_are_present_and_null() {
    let mut s = full_snapshot();
    s.session = None;
    s.observed_at = None;
    let v = serde_json::to_value(&s).unwrap();
    assert!(v.as_object().unwrap().contains_key("session"));
    assert!(v["session"].is_null());
    assert!(v["observed_at"].is_null());
    assert!(v.as_object().unwrap().contains_key("last_error"));
    assert!(v["last_error"].is_null());
}

#[test]
fn limit_view_and_team_slice_key_sets_match_ts() {
    let lv = serde_json::to_value(LimitView { label: "x".into(), utilization_pct: 1.0, resets_in_secs: 2 }).unwrap();
    assert_eq!(keys(&lv), set(&["label", "utilization_pct", "resets_in_secs"]));
    let ts = serde_json::to_value(TeamSlice { team_id: "r".into(), tokens: 1, cost_usd: 0.5 }).unwrap();
    assert_eq!(keys(&ts), set(&["team_id", "tokens", "cost_usd"]));
}

#[test]
fn threshold_band_matches_ts_string_union() {
    for (variant, s) in [(ThresholdBand::Safe, "safe"), (ThresholdBand::Warn, "warn"), (ThresholdBand::Hot, "hot"), (ThresholdBand::Braked, "braked")] {
        assert_eq!(serde_json::to_value(variant).unwrap(), Value::String(s.into()));
    }
}
