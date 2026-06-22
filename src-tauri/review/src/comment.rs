use serde::{Deserialize, Serialize};

/// Distinguishes an inline anchored comment from the optional overall-direction
/// note bundled with a revise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommentKind {
    Inline,
    Direction,
}

impl CommentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CommentKind::Inline => "inline",
            CommentKind::Direction => "direction",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "inline" => Some(CommentKind::Inline),
            "direction" => Some(CommentKind::Direction),
            _ => None,
        }
    }
}

/// A persisted comment anchored (for `Inline`) to a span in a specific artifact
/// version, or (for `Direction`) the overall-direction note for a revise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub task_id: String,
    pub artifact_path: String,
    pub anchor_text: Option<String>,
    pub anchor_offset: Option<i64>,
    pub note: String,
    pub kind: CommentKind,
    pub created_at: i64,
}

/// Input for creating a comment (id + created_at are assigned by the store).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewComment {
    pub task_id: String,
    pub artifact_path: String,
    pub anchor_text: Option<String>,
    pub anchor_offset: Option<i64>,
    pub note: String,
    pub kind: CommentKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_comment_serialises_with_kind_inline() {
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
        assert_eq!(v["kind"], "inline");
        assert_eq!(v["anchor_offset"], 42);
        assert_eq!(v["note"], "per-row, not per-batch");
    }

    #[test]
    fn direction_comment_has_null_anchor() {
        let c = Comment {
            id: "c2".into(),
            task_id: "T-1".into(),
            artifact_path: "artifacts/specs/T-1-v1.md".into(),
            anchor_text: None,
            anchor_offset: None,
            note: "overall: rewrite tasks 1+2".into(),
            kind: CommentKind::Direction,
            created_at: 1001,
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["kind"], "direction");
        assert!(v["anchor_text"].is_null());
    }

    #[test]
    fn comment_kind_parses_from_str() {
        assert_eq!(CommentKind::parse("inline"), Some(CommentKind::Inline));
        assert_eq!(CommentKind::parse("direction"), Some(CommentKind::Direction));
        assert_eq!(CommentKind::parse("nope"), None);
    }
}
