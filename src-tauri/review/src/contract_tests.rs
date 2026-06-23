//! Serde contract regression tests for the Review IPC return types `Comment`,
//! `VerdictMarker`, and the `CommentKind` enum. Lock the serialized JSON key set
//! plus enum strings against the TS interfaces. The existing tests in `comment.rs`
//! assert individual field values but not the exact key set; these add the
//! full-set assertion that catches an added, removed, or renamed field. Additive
//! test code only.

#![cfg(test)]

use crate::api::VerdictMarker;
use crate::comment::{Comment, CommentKind};
use agent_bus_core::Verdict;
use serde_json::Value;
use std::collections::BTreeSet;

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("object").keys().cloned().collect()
}
fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Locks `src/ipc/review.ts:6-15` `interface Comment`:
/// { id, task_id, artifact_path, anchor_text, anchor_offset, note, kind,
///   created_at }. Fully populated (anchors = Some) so every key appears.
#[test]
fn comment_key_set_matches_ts() {
    let c = Comment {
        id: "c1".into(),
        task_id: "T-1".into(),
        artifact_path: "artifacts/specs/T-1-v1.md".into(),
        anchor_text: Some("idempotency key".into()),
        anchor_offset: Some(42),
        note: "per-row, not per-batch".into(),
        kind: CommentKind::Inline,
        created_at: 1000,
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(
        keys(&v),
        set(&[
            "id",
            "task_id",
            "artifact_path",
            "anchor_text",
            "anchor_offset",
            "note",
            "kind",
            "created_at",
        ]),
    );
    assert_eq!(v["kind"], Value::String("inline".into()));
    assert!(v["anchor_offset"].is_number());
}

/// `anchor_text` / `anchor_offset` are `string | null` / `number | null` in TS:
/// present as `null` keys when None. Locks `src/ipc/review.ts:10-11`.
#[test]
fn comment_null_anchors_are_present_and_null() {
    let c = Comment {
        id: "c2".into(),
        task_id: "T-1".into(),
        artifact_path: "a.md".into(),
        anchor_text: None,
        anchor_offset: None,
        note: "overall".into(),
        kind: CommentKind::Direction,
        created_at: 1,
    };
    let v = serde_json::to_value(&c).unwrap();
    assert!(v.as_object().unwrap().contains_key("anchor_text"));
    assert!(v["anchor_text"].is_null());
    assert!(v["anchor_offset"].is_null());
}

/// Locks `src/ipc/review.ts:3` `type CommentKind = "inline" | "direction"`.
#[test]
fn comment_kind_matches_ts_string_union() {
    assert_eq!(serde_json::to_value(CommentKind::Inline).unwrap(), Value::String("inline".into()));
    assert_eq!(serde_json::to_value(CommentKind::Direction).unwrap(), Value::String("direction".into()));
}

/// Locks `src/ipc/review.ts:17-21` `interface VerdictMarker`:
/// { task_id, verdict, comment_count }. `verdict` is the lowercase `Verdict`
/// union; `comment_count` (Rust `usize`) is TS `number`.
#[test]
fn verdict_marker_key_set_matches_ts() {
    let m = VerdictMarker { task_id: "T-1".into(), verdict: Verdict::Revise, comment_count: 3 };
    let v = serde_json::to_value(&m).unwrap();
    assert_eq!(keys(&v), set(&["task_id", "verdict", "comment_count"]));
    assert_eq!(v["verdict"], Value::String("revise".into()));
    assert!(v["comment_count"].is_number());
}
