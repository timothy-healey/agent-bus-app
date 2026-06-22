//! The v1 command parser (D4). v1's terminal is command-driven: the user types
//! `/inject <topic>`, `/approve <task_id>`, `/brake [reason]`, etc.; this parses
//! the input into a ToolCallRequest (validated against the catalog) or a plain
//! chat line. The free-form LLM tool-use loop is v1.1 — it produces the same
//! ToolCallRequest values from a different source, so dispatch is unchanged.

use crate::catalog::ToolCatalog;
use agent_bus_core::ToolCallRequest;
use serde_json::json;

/// The outcome of parsing one input line.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    /// A recognised slash-command mapped to a tool call.
    Tool(ToolCallRequest),
    /// Plain text (no leading slash, or an unrecognised command) — handled as a
    /// chat line by the engine.
    Chat(String),
    /// A slash-command whose name isn't in the catalog or whose args are wrong.
    Error(String),
}

/// Maps the v1 canonical verbs to (tool_name, positional arg names). Only the
/// inject/approve/brake family ships in v1 (spec boundary item 10); the rest of
/// the catalog is reachable via the generic `/tool <name> {json}` form below.
fn verb_mapping(verb: &str) -> Option<(&'static str, &'static [&'static str])> {
    match verb {
        "inject" => Some(("inject_topic", &["topic"])),
        "approve" => Some(("approve_gate", &["task_id"])),
        "reject" => Some(("reject_gate", &["task_id"])),
        "revise" => Some(("revise_gate", &["task_id"])),
        "brake" => Some(("brake_on", &["reason"])),    // reason optional; see below
        "unbrake" => Some(("brake_off", &[])),
        "scale" => Some(("scale_team", &["team_id"])),
        "usage" => Some(("usage_snapshot", &[])),
        _ => None,
    }
}

/// Parse one input line against the catalog.
pub fn parse_command(input: &str, catalog: &ToolCatalog) -> Parsed {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return Parsed::Chat(trimmed.to_string());
    }
    let body = &trimmed[1..];
    let mut parts = body.splitn(2, char::is_whitespace);
    let verb = parts.next().unwrap_or("").trim();
    let rest = parts.next().unwrap_or("").trim();

    // Generic escape hatch reaching any catalog tool: `/tool <name> {json-args}`.
    if verb == "tool" {
        let mut tp = rest.splitn(2, char::is_whitespace);
        let name = tp.next().unwrap_or("").trim();
        let args_str = tp.next().unwrap_or("{}").trim();
        if catalog.by_name(name).is_none() {
            return Parsed::Error(format!("unknown tool: {name}"));
        }
        let args = serde_json::from_str(args_str)
            .unwrap_or_else(|_| json!({}));
        return Parsed::Tool(ToolCallRequest { tool_name: name.into(), args });
    }

    let Some((tool_name, arg_names)) = verb_mapping(verb) else {
        return Parsed::Error(format!("unknown command: /{verb}"));
    };
    if catalog.by_name(tool_name).is_none() {
        return Parsed::Error(format!("tool not in catalog: {tool_name}"));
    }

    // brake's reason is optional; everything else with a declared positional arg
    // requires it.
    let mut args = json!({});
    if let Some(first) = arg_names.first() {
        if rest.is_empty() {
            if tool_name == "brake_on" {
                // brake with no reason -> default reason
                args = json!({ "reason": "manual" });
            } else {
                return Parsed::Error(format!("/{verb} needs <{first}>"));
            }
        } else {
            let mut m = serde_json::Map::new();
            m.insert((*first).to_string(), json!(rest));
            args = serde_json::Value::Object(m);
        }
    }
    Parsed::Tool(ToolCallRequest { tool_name: tool_name.into(), args })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::ToolSpec;
    use serde_json::json;

    fn catalog() -> ToolCatalog {
        let spec = |n: &str, c: &str| ToolSpec {
            name: n.into(), description: "d".into(),
            input_schema: json!({"type":"object"}), supplier_context: c.into(),
        };
        ToolCatalog::new(vec![
            spec("inject_topic", "runtime"),
            spec("approve_gate", "runtime"),
            spec("brake_on", "runtime"),
            spec("brake_off", "runtime"),
            spec("usage_snapshot", "usage-telemetry"),
        ])
    }

    #[test]
    fn inject_maps_to_inject_topic_tool() {
        let p = parse_command("/inject 03-scheduling", &catalog());
        assert_eq!(
            p,
            Parsed::Tool(ToolCallRequest {
                tool_name: "inject_topic".into(),
                args: json!({ "topic": "03-scheduling" }),
            })
        );
    }

    #[test]
    fn approve_maps_to_approve_gate_with_task_id() {
        let p = parse_command("/approve T-041", &catalog());
        assert_eq!(
            p,
            Parsed::Tool(ToolCallRequest {
                tool_name: "approve_gate".into(),
                args: json!({ "task_id": "T-041" }),
            })
        );
    }

    #[test]
    fn brake_without_reason_defaults_to_manual() {
        let p = parse_command("/brake", &catalog());
        assert_eq!(
            p,
            Parsed::Tool(ToolCallRequest { tool_name: "brake_on".into(), args: json!({"reason": "manual"}) })
        );
    }

    #[test]
    fn brake_with_reason_carries_it() {
        let p = parse_command("/brake rate-limit", &catalog());
        assert_eq!(p, Parsed::Tool(ToolCallRequest { tool_name: "brake_on".into(), args: json!({"reason": "rate-limit"}) }));
    }

    #[test]
    fn unbrake_maps_to_brake_off_no_args() {
        let p = parse_command("/unbrake", &catalog());
        assert_eq!(p, Parsed::Tool(ToolCallRequest { tool_name: "brake_off".into(), args: json!({}) }));
    }

    #[test]
    fn plain_text_is_chat() {
        assert_eq!(parse_command("what is T-042 doing?", &catalog()), Parsed::Chat("what is T-042 doing?".into()));
    }

    #[test]
    fn unknown_command_is_error() {
        assert_eq!(parse_command("/frobnicate x", &catalog()), Parsed::Error("unknown command: /frobnicate".into()));
    }

    #[test]
    fn approve_without_task_id_is_error() {
        assert_eq!(parse_command("/approve", &catalog()), Parsed::Error("/approve needs <task_id>".into()));
    }

    #[test]
    fn generic_tool_form_reaches_any_catalog_tool() {
        let p = parse_command("/tool usage_snapshot {}", &catalog());
        assert_eq!(p, Parsed::Tool(ToolCallRequest { tool_name: "usage_snapshot".into(), args: json!({}) }));
    }
}
