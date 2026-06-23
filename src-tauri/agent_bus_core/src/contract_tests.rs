//! Serde contract regression tests for the shared-kernel wire types.
//!
//! These lock the JSON shape the kernel's `Serialize` impls produce against the
//! TypeScript interfaces the frontend declares for the same values. They assert
//! the *exact* key set (so a renamed / added / removed field, or a casing flip,
//! fails `cargo test`) and the *exact* enum string / tag representation. They
//! deliberately do not rely on round-trip alone (a symmetric rename round-trips
//! fine but still breaks the TS contract).
//!
//! Additive test code only — no production behaviour is touched.

#![cfg(test)]

use crate::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// Collect the top-level object keys of a `serde_json::Value` as a sorted set.
fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object()
        .expect("expected a JSON object")
        .keys()
        .cloned()
        .collect()
}

/// Build a BTreeSet<String> from a slice of &str literals.
fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

// --- newtype IDs / PathBuf serialise as bare strings ------------------------

/// Locks: every newtype id is TS `string` (e.g. `src/ipc/runtime.ts:13`
/// `Task.id: string`, `src/ipc/workspace.ts:4` `Project.id: string`).
#[test]
fn newtype_ids_serialise_as_bare_strings() {
    assert_eq!(serde_json::to_value(ProjectId("p1".into())).unwrap(), json!("p1"));
    assert_eq!(serde_json::to_value(TaskId("T-1".into())).unwrap(), json!("T-1"));
    assert_eq!(serde_json::to_value(TeamId("research".into())).unwrap(), json!("research"));
    assert_eq!(serde_json::to_value(PipelineId("pipe".into())).unwrap(), json!("pipe"));
    assert_eq!(
        serde_json::to_value(ArtifactPath(std::path::PathBuf::from("/a/b.md"))).unwrap(),
        json!("/a/b.md")
    );
}

// --- Verdict: TS string union "approve" | "revise" | "reject" ---------------

/// Locks `src/ipc/review.ts:4`: `type Verdict = "approve" | "revise" | "reject"`.
#[test]
fn verdict_matches_ts_string_union() {
    assert_eq!(serde_json::to_value(Verdict::Approve).unwrap(), json!("approve"));
    assert_eq!(serde_json::to_value(Verdict::Revise).unwrap(), json!("revise"));
    assert_eq!(serde_json::to_value(Verdict::Reject).unwrap(), json!("reject"));
}

// --- RunnerKind: TS "claude-cli" | "anthropic-api" (kebab) ------------------

/// Locks `src/ipc/pipeline.ts:3`: `type RunnerKind = "claude-cli" | "anthropic-api"`.
#[test]
fn runner_kind_matches_ts_kebab_union() {
    assert_eq!(serde_json::to_value(RunnerKind::ClaudeCli).unwrap(), json!("claude-cli"));
    assert_eq!(serde_json::to_value(RunnerKind::AnthropicApi).unwrap(), json!("anthropic-api"));
}

// --- EffortMode: internally-tagged on `mode`, kebab variants ----------------

/// Locks `src/ipc/pipeline.ts:5-10`: the `EffortMode` discriminated union tagged
/// on `mode` with kebab variant names; the `custom` variant adds `budget_tokens`.
#[test]
fn effort_mode_matches_ts_discriminated_union() {
    // Unit variants: object with exactly { mode }.
    for (variant, tag) in [
        (EffortMode::Off, "off"),
        (EffortMode::Standard, "standard"),
        (EffortMode::ExtendedLow, "extended-low"),
        (EffortMode::ExtendedHigh, "extended-high"),
    ] {
        let v = serde_json::to_value(variant).unwrap();
        assert_eq!(keys(&v), set(&["mode"]), "variant {tag} keys");
        assert_eq!(v["mode"], json!(tag));
    }
    // Custom variant: { mode: "custom", budget_tokens: number }.
    let custom = serde_json::to_value(EffortMode::Custom { budget_tokens: 16000 }).unwrap();
    assert_eq!(keys(&custom), set(&["mode", "budget_tokens"]));
    assert_eq!(custom["mode"], json!("custom"));
    assert_eq!(custom["budget_tokens"], json!(16000));
}

// --- ToolCallRequest: TS { tool_name, args } --------------------------------

/// Locks `src/ipc/terminal.ts:5-8` `interface ToolCallRequest { tool_name; args }`.
#[test]
fn tool_call_request_key_set_matches_ts() {
    let req = ToolCallRequest { tool_name: "inject_topic".into(), args: json!({"topic": "x"}) };
    let v = serde_json::to_value(&req).unwrap();
    assert_eq!(keys(&v), set(&["tool_name", "args"]));
    assert_eq!(v["tool_name"], json!("inject_topic"));
}

// --- ToolCallResult: tagged on `status`, snake_case variants ----------------

/// Locks `src/ipc/terminal.ts:10-12`: the `ToolCallResult` discriminated union
/// tagged on `status` — `{ status: "ok"; result }` | `{ status: "err"; error }`.
#[test]
fn tool_call_result_matches_ts_discriminated_union() {
    let ok = serde_json::to_value(ToolCallResult::Ok { result: json!({"task_id": "T-1"}) }).unwrap();
    assert_eq!(keys(&ok), set(&["status", "result"]));
    assert_eq!(ok["status"], json!("ok"));

    let err = serde_json::to_value(ToolCallResult::Err { error: "boom".into() }).unwrap();
    assert_eq!(keys(&err), set(&["status", "error"]));
    assert_eq!(err["status"], json!("err"));
}
