//! Serde contract regression tests for the Conversational Control IPC return
//! type `Conversation` and its nested value types `Turn`, `ToolCall`, and the
//! `Role` enum. Lock the serialized JSON key set + enum strings against the TS
//! interfaces. The TS file is `src/ipc/terminal.ts`. Additive only.

#![cfg(test)]

use crate::conversation::Conversation;
use crate::turn::{Role, ToolCall, Turn};
use agent_bus_core::{ToolCallRequest, ToolCallResult};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("object").keys().cloned().collect()
}
fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn full_tool_call() -> ToolCall {
    ToolCall {
        request: ToolCallRequest { tool_name: "inject_topic".into(), args: json!({"topic": "x"}) },
        // Some so the TS `ToolCallResult | null` field is exercised as a value.
        result: Some(ToolCallResult::Ok { result: json!({"task_id": "T-1"}) }),
    }
}

fn full_conversation() -> Conversation {
    let mut c = Conversation::new("proj-1", "sess-1", 1_700_000_000);
    c.last_message_at = 1_700_000_100;
    c.turns = vec![
        Turn::user("hello", 1_700_000_000),
        Turn::assistant("on it", vec![full_tool_call()], 1_700_000_100),
    ];
    c.summary_of_prior_sessions = Some("earlier we discussed X".into());
    c
}

/// Locks `src/ipc/terminal.ts:26-34` `interface Conversation`:
/// { project_id, session_id, started_at, last_message_at, turns,
///   summary_of_prior_sessions, history_budget_tokens }.
#[test]
fn conversation_key_set_matches_ts() {
    let v = serde_json::to_value(full_conversation()).unwrap();
    assert_eq!(
        keys(&v),
        set(&[
            "project_id",
            "session_id",
            "started_at",
            "last_message_at",
            "turns",
            "summary_of_prior_sessions",
            "history_budget_tokens",
        ]),
    );
    assert!(v["turns"].is_array());
    assert!(v["summary_of_prior_sessions"].is_string());
    assert!(v["history_budget_tokens"].is_number());
}

/// `summary_of_prior_sessions` is `string | null` in TS: present as `null` key
/// when None. Locks `src/ipc/terminal.ts:32`.
#[test]
fn conversation_null_summary_is_present_and_null() {
    let c = Conversation::new("p", "s", 1);
    let v = serde_json::to_value(&c).unwrap();
    assert!(v.as_object().unwrap().contains_key("summary_of_prior_sessions"));
    assert!(v["summary_of_prior_sessions"].is_null());
}

/// Locks `src/ipc/terminal.ts:19-24` `interface Turn`:
/// { role, text, tool_calls, at }.
#[test]
fn turn_key_set_matches_ts() {
    let t = Turn::assistant("done", vec![full_tool_call()], 100);
    let v = serde_json::to_value(&t).unwrap();
    assert_eq!(keys(&v), set(&["role", "text", "tool_calls", "at"]));
    assert_eq!(v["role"], Value::String("assistant".into()));
    assert!(v["tool_calls"].is_array());
}

/// Locks `src/ipc/terminal.ts:14-17` `interface ToolCall { request; result }`.
/// `result` is `ToolCallResult | null`; with Some it is the tagged object.
#[test]
fn tool_call_key_set_matches_ts() {
    let v = serde_json::to_value(full_tool_call()).unwrap();
    assert_eq!(keys(&v), set(&["request", "result"]));
    // nested request keys (terminal.ts:5-8)
    assert_eq!(keys(&v["request"]), set(&["tool_name", "args"]));
    // nested result is tagged on `status` (terminal.ts:10-12)
    assert_eq!(v["result"]["status"], Value::String("ok".into()));
}

/// `result: null` is present (TS `ToolCallResult | null`). Locks `terminal.ts:16`.
#[test]
fn tool_call_null_result_is_present_and_null() {
    let tc = ToolCall {
        request: ToolCallRequest { tool_name: "approve_gate".into(), args: json!({}) },
        result: None,
    };
    let v = serde_json::to_value(&tc).unwrap();
    assert!(v.as_object().unwrap().contains_key("result"));
    assert!(v["result"].is_null());
}

/// Locks `src/ipc/terminal.ts:3` `type Role = "user" | "assistant"`.
#[test]
fn role_matches_ts_string_union() {
    assert_eq!(serde_json::to_value(Role::User).unwrap(), Value::String("user".into()));
    assert_eq!(serde_json::to_value(Role::Assistant).unwrap(), Value::String("assistant".into()));
}
