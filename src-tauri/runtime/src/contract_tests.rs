//! Serde contract regression tests for the Runtime IPC return types `Task`,
//! `TaskState`, and `BrakeState`. Lock the serialized JSON key set + enum
//! strings against the TS interfaces the frontend declares. Additive only.

#![cfg(test)]

use crate::brake::BrakeState;
use crate::task::{Task, TaskState};
use agent_bus_core::TaskId;
use serde_json::Value;
use std::collections::BTreeSet;

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("object").keys().cloned().collect()
}
fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// A fully-populated Task: every Option = Some so all keys appear.
fn full_task() -> Task {
    let mut t = Task::injected(
        "proj-1".into(),
        "pipe".into(),
        "research".into(),
        "topic".into(),
        Some("/repo".into()),
        1_700_000_000,
    );
    t.id = TaskId("T-1".into());
    t.target_scope = Some("crates/runtime".into());
    t.parent_artifact = Some("artifacts/specs/T-1-v1.md".into());
    t.review_artifact = Some("artifacts/review/T-1-v1.md".into());
    t.state = TaskState::Gated;
    t.group_id = Some("G-1".into());
    t.lane = Some("lane-a".into());
    t.join_target = Some("join-1".into());
    t
}

/// Locks `src/ipc/runtime.ts:12-26` `interface Task`:
/// { id, project_id, pipeline, topic, target_repo, target_scope, current_stage,
///   state, attempts, parent_artifact, review_artifact, created_at, updated_at }.
#[test]
fn task_key_set_matches_ts() {
    let v = serde_json::to_value(full_task()).unwrap();
    assert_eq!(
        keys(&v),
        set(&[
            "id",
            "project_id",
            "pipeline",
            "topic",
            "target_repo",
            "target_scope",
            "current_stage",
            "state",
            "attempts",
            "parent_artifact",
            "review_artifact",
            "created_at",
            "updated_at",
            "group_id",
            "lane",
            "join_target",
        ]),
    );
    // newtype id → bare string; counters → numbers.
    assert!(v["id"].is_string());
    assert!(v["attempts"].is_number());
    assert!(v["created_at"].is_number());
}

/// Optional fields, when None, must still appear as `null` keys (TS declares
/// them `string | null`, not optional). Locks `src/ipc/runtime.ts:17-23`.
#[test]
fn task_none_options_are_present_and_null() {
    let t = Task::injected("p".into(), "pipe".into(), "research".into(), "topic".into(), None, 1);
    let v = serde_json::to_value(&t).unwrap();
    for k in ["target_repo", "target_scope", "parent_artifact", "review_artifact"] {
        assert!(v.as_object().unwrap().contains_key(k), "missing key {k}");
        assert!(v[k].is_null(), "expected {k} null");
    }
}

/// Locks `src/ipc/runtime.ts:3-10` `type TaskState` snake_case string union.
#[test]
fn task_state_matches_ts_string_union() {
    let cases = [
        (TaskState::Queued, "queued"),
        (TaskState::Running, "running"),
        (TaskState::Gated, "gated"),
        (TaskState::Revising, "revising"),
        (TaskState::NeedsHuman, "needs_human"),
        (TaskState::Done, "done"),
        (TaskState::Braked, "braked"),
    ];
    for (variant, s) in cases {
        assert_eq!(serde_json::to_value(variant).unwrap(), Value::String(s.into()));
    }
}

/// Locks `src/ipc/runtime.ts:28-31` `interface BrakeState { on; reason }`.
/// `reason` is `string | null` and present in both Some and None forms.
#[test]
fn brake_state_key_set_matches_ts() {
    let on = BrakeState { on: true, reason: Some("rate-limit".into()) };
    let v = serde_json::to_value(&on).unwrap();
    assert_eq!(keys(&v), set(&["on", "reason"]));
    assert!(v["on"].is_boolean());
    assert!(v["reason"].is_string());

    let off = BrakeState { on: false, reason: None };
    let v2 = serde_json::to_value(&off).unwrap();
    assert_eq!(keys(&v2), set(&["on", "reason"]));
    assert!(v2["reason"].is_null());
}
