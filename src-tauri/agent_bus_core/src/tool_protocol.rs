use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Describes an app-tool a supplier context publishes as part of its OHS.
/// The Conversational Control context consumes the union of all suppliers'
/// ToolSpec values to build the god terminal's tool catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Tool name as Claude sees it (e.g. "inject_topic", "approve_gate").
    pub name: String,
    /// Human-readable description (shown to Claude).
    pub description: String,
    /// JSON Schema for the tool's arguments.
    pub input_schema: Value,
    /// The supplier context that owns this tool. Contract: one of the supplier
    /// slugs from DOMAIN.md — "pipeline-authoring", "runtime", "review",
    /// "usage-telemetry", "runners", "workspace". These slugs must stay stable:
    /// Conversational Control's tool catalog (Plan 6) groups tools by this value.
    /// Kept a String for v1; see the v1.1 note below re: promoting to an enum.
    pub supplier_context: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub tool_name: String,
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolCallResult {
    Ok { result: Value },
    Err { error: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_spec_round_trips() {
        let spec = ToolSpec {
            name: "inject_topic".into(),
            description: "Inject a new topic into the research team's inbox.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "topic": { "type": "string" } },
                "required": ["topic"]
            }),
            supplier_context: "runtime".into(),
        };
        let s = serde_json::to_string(&spec).unwrap();
        let back: ToolSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(spec, back);
    }

    #[test]
    fn tool_call_result_serialises_with_status_tag() {
        let ok = ToolCallResult::Ok { result: json!({"task_id": "T-042"}) };
        let err = ToolCallResult::Err { error: "rate-limited".into() };

        let ok_json = serde_json::to_string(&ok).unwrap();
        let err_json = serde_json::to_string(&err).unwrap();

        assert!(ok_json.contains("\"status\":\"ok\""));
        assert!(err_json.contains("\"status\":\"err\""));
    }
}
