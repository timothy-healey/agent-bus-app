//! Argument validation against a tool's published JSON `input_schema` (T1).
//!
//! The terminal validates a `{tool, args}` request's `args` against the catalog
//! tool's DERIVED `input_schema` BEFORE dispatch. This is the ACL seal: the
//! kernel validates only the published JSON Schema — it never learns the supplier
//! arg TYPES (those stay owned by each supplier). A miss yields a SPECIFIC,
//! human-readable message naming the offending property, which the slash path
//! surfaces to the user and the agentic loop re-prompts the model with.

use serde_json::Value;

/// Validate `args` against a tool's JSON `input_schema`. Returns `Ok(())` when
/// the args satisfy the schema, or a specific human-readable error otherwise.
///
/// The error names the offending property (e.g. "`task_id` is required",
/// "`budget` has the wrong type"). A schema that cannot be compiled is treated
/// as "accept" (a healthy catalog never produces one; the suppliers' derived
/// schemas are always valid) so a catalog bug can never silently block dispatch.
pub fn validate_args(input_schema: &Value, args: &Value) -> Result<(), String> {
    let validator = match jsonschema::validator_for(input_schema) {
        Ok(v) => v,
        // An uncompilable schema is a catalog-construction bug, not a user error;
        // do not block dispatch on it (matches `by_name`-only behaviour before T1).
        Err(_) => return Ok(()),
    };
    // Collect the first error and render it as a property-named message. We
    // surface ONE error (the first) so the repair re-prompt is crisp.
    if let Some(err) = validator.iter_errors(args).next() {
        return Err(render_error(&err));
    }
    Ok(())
}

/// Render a `ValidationError` into a crisp, property-named message. We special-case
/// the two dominant misses — a missing required prop and a wrong type — because
/// those are what the model (and the operator) actually need named.
fn render_error(err: &jsonschema::ValidationError) -> String {
    use jsonschema::error::ValidationErrorKind;
    // `instance_path` points at the offending value; for a `required` miss it is
    // the parent object, so the property name lives in the kind instead.
    let path = err.instance_path().to_string();
    let prop = path.trim_start_matches('/');
    match err.kind() {
        ValidationErrorKind::Required { property } => {
            let name = property.as_str().unwrap_or("a required field");
            format!("`{name}` is required")
        }
        ValidationErrorKind::Type { .. } => {
            if prop.is_empty() {
                format!("the arguments have the wrong type: {err}")
            } else {
                format!("`{prop}` has the wrong type: {err}")
            }
        }
        _ => {
            if prop.is_empty() {
                err.to_string()
            } else {
                format!("`{prop}`: {err}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gate_schema() -> Value {
        json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "required": ["task_id"],
            "properties": { "task_id": { "type": "string" } }
        })
    }

    fn budget_schema() -> Value {
        json!({
            "type": "object",
            "required": ["budget"],
            "properties": { "budget": { "type": "integer" } }
        })
    }

    fn inject_schema() -> Value {
        json!({
            "type": "object",
            "required": ["topic"],
            "properties": {
                "topic": { "type": "string" },
                "target_repo": { "type": ["string", "null"] }
            }
        })
    }

    #[test]
    fn valid_args_pass() {
        assert!(validate_args(&gate_schema(), &json!({ "task_id": "T-1" })).is_ok());
        assert!(validate_args(&inject_schema(), &json!({ "topic": "x" })).is_ok());
        // optional field present is still fine
        assert!(validate_args(&inject_schema(), &json!({ "topic": "x", "target_repo": "/r" })).is_ok());
        // optional field omitted is fine
        assert!(validate_args(&inject_schema(), &json!({ "topic": "x" })).is_ok());
    }

    #[test]
    fn missing_required_prop_is_rejected_with_a_specific_message() {
        let err = validate_args(&gate_schema(), &json!({})).unwrap_err();
        assert!(err.contains("task_id"), "message must name the missing prop: {err}");
        assert!(err.to_lowercase().contains("required"), "message must say required: {err}");
    }

    #[test]
    fn wrong_typed_prop_is_rejected_with_a_specific_message() {
        let err = validate_args(&budget_schema(), &json!({ "budget": "lots" })).unwrap_err();
        assert!(err.contains("budget"), "message must name the bad prop: {err}");
    }

    #[test]
    fn an_uncompilable_schema_does_not_block_dispatch() {
        // A garbage schema is a catalog bug, not a user error -> accept (no late block).
        let bad = json!({ "type": 12345 });
        assert!(validate_args(&bad, &json!({ "anything": true })).is_ok());
    }

    #[test]
    fn empty_object_schema_accepts_anything() {
        assert!(validate_args(&json!({ "type": "object" }), &json!({})).is_ok());
        assert!(validate_args(&json!({ "type": "object" }), &json!({ "x": 1 })).is_ok());
    }
}
