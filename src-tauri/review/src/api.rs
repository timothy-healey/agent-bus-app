use crate::comment::Comment;
use crate::comment::{CommentKind, NewComment};
use crate::store::CommentStore;
use agent_bus_core::tool_protocol::ToolSpec;
use agent_bus_core::Verdict;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

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
    fn tools_publishes_review_ohs_under_review_context() {
        let specs = tools();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"add_comment"));
        assert!(names.contains(&"list_comments"));
        assert!(names.contains(&"record_verdict"));
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
