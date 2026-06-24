//! Serde contract regression test for the Workspace IPC return type `Project`.
//! Locks the serialized JSON key set + value kinds against the TS interface the
//! frontend declares, so a field rename / add / remove / casing flip fails
//! `cargo test`. Additive test code only.

#![cfg(test)]

use crate::project::Project;
use agent_bus_core::PipelineId;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("object").keys().cloned().collect()
}
fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Locks `src/ipc/workspace.ts:3-10` `interface Project`:
/// { id, name, root_path, target_repo, active_pipeline_id, created_at, updated_at }.
/// `active_pipeline_id` and `target_repo` are `string | null` (TS) → here
/// `Option<_>`, and must still appear as keys when `Some` (serde serialises them).
#[test]
fn project_key_set_and_string_ids_match_ts() {
    // Fully populated: active_pipeline_id + target_repo = Some so every key appears.
    let mut p = Project::new("My Project".into(), PathBuf::from("/repos/app"), 1_700_000_000);
    p.active_pipeline_id = Some(PipelineId("pipe-1".into()));
    p.target_repo = Some("/repos/target".into());
    p.skill_sources = vec!["/repos/app/.claude".into()];

    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(
        keys(&v),
        set(&["id", "name", "root_path", "target_repo", "skill_sources", "active_pipeline_id", "created_at", "updated_at"]),
    );
    assert!(v["skill_sources"].is_array());

    // Newtype id + PathBuf serialise as bare strings (TS `string`).
    assert!(v["id"].is_string());
    assert!(v["root_path"].is_string());
    assert_eq!(v["root_path"], Value::String("/repos/app".into()));
    assert!(v["target_repo"].is_string()); // Some(String) → bare string
    assert!(v["active_pipeline_id"].is_string()); // Some(PipelineId) → bare string
    assert!(v["created_at"].is_number());
    assert!(v["updated_at"].is_number());
}

/// With `active_pipeline_id = None` the key must still be present and `null`
/// (TS declares it `string | null`, not optional).
#[test]
fn project_null_active_pipeline_id_is_present_and_null() {
    let p = Project::new("X".into(), PathBuf::from("/p"), 1);
    let v = serde_json::to_value(&p).unwrap();
    assert!(v.as_object().unwrap().contains_key("active_pipeline_id"));
    assert!(v["active_pipeline_id"].is_null());
}
