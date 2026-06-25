use crate::comment::Comment;
use crate::comment::{CommentKind, NewComment};
use crate::reanchor::{reanchor_comments as reanchor_project, ReanchoredComment};
use crate::store::CommentStore;
use agent_bus_core::tool_protocol::ToolSpec;
use agent_bus_core::Verdict;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

/// Per-tool argument types for Review's OHS tools (T1). Each tool's
/// `input_schema` is DERIVED from these via `schemars` (one source of truth —
/// the dispatcher deserializes the same struct). Owned by the supplier; the
/// agentic loop validates only the published JSON schema, never these types.
pub mod args {
    use super::{Deserialize, JsonSchema, Verdict};

    /// `add_comment` — add an inline or direction comment to a task's artifact.
    /// `kind` is loosely a string ("inline"/"direction") matching the command.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
    pub struct AddCommentArgs {
        pub task_id: String,
        pub artifact_path: String,
        pub note: String,
        #[serde(default)]
        pub anchor_text: Option<String>,
        #[serde(default)]
        pub anchor_offset: Option<i64>,
        #[serde(default)]
        pub kind: Option<String>,
    }

    /// `list_comments` — list all comments for a task.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
    pub struct ListCommentsArgs {
        pub task_id: String,
    }

    /// `reanchor_comments` — re-anchor a task's comments onto a viewed version.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
    pub struct ReanchorCommentsArgs {
        pub task_id: String,
        pub version_markdown: String,
    }

    /// `record_verdict` — record a review verdict (approve/revise/reject).
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
    pub struct RecordVerdictArgs {
        pub task_id: String,
        pub verdict: Verdict,
    }
}

/// Managed state for the Review context.
pub struct ReviewState {
    pub comments: Arc<CommentStore>,
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The conformist verdict marker Review records when the operator decides at a
/// gate. It does NOT change Task state (Runtime's `*_gate` commands do that);
/// this is the Review-side audit that a verdict was emitted in Runtime's
/// vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictMarker {
    pub task_id: String,
    pub verdict: Verdict,
    pub comment_count: usize,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn add_comment(
    state: tauri::State<'_, ReviewState>,
    task_id: String,
    artifact_path: String,
    note: String,
    anchor_text: Option<String>,
    anchor_offset: Option<i64>,
    kind: Option<String>,
) -> Result<Comment, String> {
    let kind = match kind.as_deref() {
        None => CommentKind::Inline,
        Some(s) => CommentKind::parse(s).ok_or_else(|| format!("bad kind: {s}"))?,
    };
    let nc = NewComment {
        task_id,
        artifact_path,
        anchor_text,
        anchor_offset,
        note,
        kind,
    };
    state
        .comments
        .insert(nc, now_unix())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_comments(
    state: tauri::State<'_, ReviewState>,
    task_id: String,
) -> Result<Vec<Comment>, String> {
    state
        .comments
        .list_for_task(&task_id)
        .await
        .map_err(|e| e.to_string())
}

/// Re-anchor a task's comments onto a viewed artifact version (B1). Pure,
/// read-derived: lists the stored comments, parses `addressed: <id>` markers
/// from `version_markdown`, and returns each comment with its derived status
/// (`open`/`addressed`) and effective offset. Persists nothing; the Conformist
/// seam to Runtime is untouched (no verdict, no Task-state change).
#[tauri::command(rename_all = "snake_case")]
pub async fn reanchor_comments(
    state: tauri::State<'_, ReviewState>,
    task_id: String,
    version_markdown: String,
) -> Result<Vec<ReanchoredComment>, String> {
    let comments = state
        .comments
        .list_for_task(&task_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(reanchor_project(&comments, &version_markdown))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_comment(
    state: tauri::State<'_, ReviewState>,
    comment_id: String,
) -> Result<(), String> {
    state
        .comments
        .delete(&comment_id)
        .await
        .map_err(|e| e.to_string())
}

/// Records the verdict marker (Review-side audit) and returns it. The frontend
/// then calls the Runtime gate command that actually transitions the Task —
/// keeping Review Conformist to Runtime's state machine.
#[tauri::command(rename_all = "snake_case")]
pub async fn record_verdict(
    state: tauri::State<'_, ReviewState>,
    task_id: String,
    verdict: Verdict,
) -> Result<VerdictMarker, String> {
    let comments = state
        .comments
        .list_for_task(&task_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(VerdictMarker {
        task_id,
        verdict,
        comment_count: comments.len(),
    })
}

/// The Review OHS tool catalog (consumed by Conversational Control in Plan 6).
pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "add_comment".into(),
            description: "Add an inline or direction comment to a task's artifact.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"},
                    "artifact_path": {"type": "string"},
                    "note": {"type": "string"},
                    "anchor_text": {"type": ["string", "null"]},
                    "anchor_offset": {"type": ["integer", "null"]},
                    "kind": {"type": "string", "enum": ["inline", "direction"]}
                },
                "required": ["task_id", "artifact_path", "note"]
            }),
            supplier_context: "review".into(),
        },
        ToolSpec {
            name: "list_comments".into(),
            description: "List all comments for a task.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"task_id": {"type": "string"}},
                "required": ["task_id"]
            }),
            supplier_context: "review".into(),
        },
        ToolSpec {
            name: "reanchor_comments".into(),
            description: "Re-anchor a task's comments onto a viewed artifact version using \
                          <!-- addressed: <comment-id> --> markers; returns each comment with \
                          a derived status (open/addressed) and effective offset."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"},
                    "version_markdown": {"type": "string"}
                },
                "required": ["task_id", "version_markdown"]
            }),
            supplier_context: "review".into(),
        },
        ToolSpec {
            name: "record_verdict".into(),
            description: "Record a review verdict (approve/revise/reject) for a task.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"},
                    "verdict": {"type": "string", "enum": ["approve", "revise", "reject"]}
                },
                "required": ["task_id", "verdict"]
            }),
            supplier_context: "review".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_comment_args_round_trip_required_and_optional() {
        let a: args::AddCommentArgs = serde_json::from_value(json!({
            "task_id": "T-1", "artifact_path": "spec.md", "note": "fix this"
        })).unwrap();
        assert_eq!(a.task_id, "T-1");
        assert_eq!(a.anchor_offset, None);
        assert_eq!(a.kind, None);
        let b: args::AddCommentArgs = serde_json::from_value(json!({
            "task_id": "T-1", "artifact_path": "spec.md", "note": "n",
            "anchor_text": "foo", "anchor_offset": 12, "kind": "direction"
        })).unwrap();
        assert_eq!(b.anchor_offset, Some(12));
        assert_eq!(b.kind.as_deref(), Some("direction"));
        // missing the required `note` is rejected.
        assert!(serde_json::from_value::<args::AddCommentArgs>(
            json!({ "task_id": "T-1", "artifact_path": "spec.md" })).is_err());
    }

    #[test]
    fn record_verdict_args_round_trip_in_runtime_vocabulary() {
        let a: args::RecordVerdictArgs =
            serde_json::from_value(json!({ "task_id": "T-1", "verdict": "revise" })).unwrap();
        assert_eq!(a.verdict, Verdict::Revise);
        // a non-vocabulary verdict is rejected.
        assert!(serde_json::from_value::<args::RecordVerdictArgs>(
            json!({ "task_id": "T-1", "verdict": "maybe" })).is_err());
    }

    #[test]
    fn reanchor_and_list_args_require_their_fields() {
        let r: args::ReanchorCommentsArgs =
            serde_json::from_value(json!({ "task_id": "T-1", "version_markdown": "# v" })).unwrap();
        assert_eq!(r.version_markdown, "# v");
        assert!(serde_json::from_value::<args::ReanchorCommentsArgs>(json!({ "task_id": "T-1" })).is_err());
        let l: args::ListCommentsArgs = serde_json::from_value(json!({ "task_id": "T-1" })).unwrap();
        assert_eq!(l.task_id, "T-1");
    }

    #[test]
    fn tools_publishes_review_ohs_under_review_context() {
        let specs = tools();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"add_comment"));
        assert!(names.contains(&"list_comments"));
        assert!(names.contains(&"record_verdict"));
        assert!(names.contains(&"reanchor_comments"));
        assert!(specs.iter().all(|s| s.supplier_context == "review"));
    }

    #[test]
    fn verdict_marker_serialises_in_runtime_vocabulary() {
        let m = VerdictMarker {
            task_id: "T-1".into(),
            verdict: agent_bus_core::Verdict::Revise,
            comment_count: 2,
        };
        let v = serde_json::to_value(&m).unwrap();
        // Conformist: verdict uses Runtime's lowercase vocabulary.
        assert_eq!(v["verdict"], "revise");
        assert_eq!(v["comment_count"], 2);
    }
}
