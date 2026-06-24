//! Serde contract regression test for the IPC type `SkillEntry`. Locks the
//! serialized JSON key set + value kinds against the TS interface the frontend
//! declares (`src/ipc/skills.ts`), so a field rename / add / remove / casing
//! flip fails `cargo test`. Same discipline as the Workspace `Project` contract.

#![cfg(test)]

use crate::model::{SkillEntry, SkillKind, SkillSource};
use serde_json::Value;
use std::collections::BTreeSet;

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("object").keys().cloned().collect()
}
fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Locks `interface SkillEntry`:
/// { name, kind, namespace, description, verbs, source, qualified }.
#[test]
fn skill_entry_key_set_and_value_kinds_match_ts() {
    let e = SkillEntry {
        name: "ddd-council".into(),
        kind: SkillKind::Skill,
        namespace: Some("ddd-council".into()),
        description: "DDD council".into(),
        verbs: vec!["vet".into(), "critique".into()],
        source: SkillSource::Project,
        qualified: true,
    };
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(
        keys(&v),
        set(&["name", "kind", "namespace", "description", "verbs", "source", "qualified"]),
    );
    assert!(v["name"].is_string());
    assert!(v["namespace"].is_string()); // Some(String) → bare string
    assert!(v["description"].is_string());
    assert!(v["verbs"].is_array());
    assert!(v["qualified"].is_boolean());
    // Enums serialize as lowercase string tags.
    assert_eq!(v["kind"], Value::String("skill".into()));
    assert_eq!(v["source"], Value::String("project".into()));
}

/// kind=Command + namespace=None: the key must still be present and `null`
/// (TS declares it `string | null`, not optional).
#[test]
fn command_kind_and_null_namespace_serialize_as_expected() {
    let e = SkillEntry {
        name: "init-session".into(),
        kind: SkillKind::Command,
        namespace: None,
        description: String::new(),
        verbs: vec![],
        source: SkillSource::Global,
        qualified: false,
    };
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["kind"], Value::String("command".into()));
    assert_eq!(v["source"], Value::String("global".into()));
    assert!(v.as_object().unwrap().contains_key("namespace"));
    assert!(v["namespace"].is_null());
}

/// `SkillEntry` round-trips through serde (the frontend sends nothing back, but
/// the symmetry guards against an asymmetric custom impl creeping in).
#[test]
fn skill_entry_round_trips() {
    let e = SkillEntry {
        name: "x".into(),
        kind: SkillKind::Skill,
        namespace: None,
        description: "d".into(),
        verbs: vec!["a".into()],
        source: SkillSource::Global,
        qualified: false,
    };
    let s = serde_json::to_string(&e).unwrap();
    let back: SkillEntry = serde_json::from_str(&s).unwrap();
    assert_eq!(e, back);
}
