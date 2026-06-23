//! Pure, read-derived comment re-anchoring for the Review context (B1).
//!
//! When a writer addresses a comment in a later artifact version they embed a
//! marker `<!-- addressed: <comment-id> -->` into the new markdown. These pure
//! functions parse those markers and project the stored structured comments
//! onto the viewed version: an addressed comment is re-anchored to the marker's
//! position (status `addressed`); an unaddressed prior comment carries over
//! (status `open`). Nothing is persisted — the projection is derived on read
//! from (stored comments) x (version markdown). The marker convention is part
//! of Review's ubiquitous language (see DOMAIN.md). Distinct from B2's
//! display-only line-diff: B2 classifies line identity; B1 re-anchors comment
//! identity across versions.

use crate::comment::{Comment, CommentKind};
use serde::{Deserialize, Serialize};

/// One parsed `<!-- addressed: <comment-id> -->` marker and its byte offset
/// (start of the marker) within the version markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressedMarker {
    pub comment_id: String,
    pub offset: usize,
}

/// Parse all `<!-- addressed: <comment-id> -->` markers from a version's
/// markdown, in source order. Tolerates arbitrary internal whitespace:
/// `<!--addressed:c1-->`, `<!-- addressed:  c1  -->` both parse to `c1`.
/// Ignores any other HTML comment. Returns each marker's start byte offset.
pub fn parse_addressed_markers(markdown: &str) -> Vec<AddressedMarker> {
    let mut out = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = markdown[search_from..].find("<!--") {
        let start = search_from + rel;
        let Some(end_rel) = markdown[start..].find("-->") else {
            break;
        };
        let end = start + end_rel; // index of "-->"
        let inner = markdown[start + 4..end].trim();
        if let Some(rest) = inner.strip_prefix("addressed:") {
            let id = rest.trim();
            if !id.is_empty() {
                out.push(AddressedMarker {
                    comment_id: id.to_string(),
                    offset: start,
                });
            }
        }
        search_from = end + 3; // past "-->"
    }
    out
}

/// Derived status of a comment relative to the viewed version. `Open` = still
/// carried over (not yet addressed in this version); `Addressed` = the version
/// contains an `addressed: <id>` marker for it (re-anchored to the marker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommentStatus {
    Open,
    Addressed,
}

/// A stored comment projected onto the viewed version: the structured comment
/// plus its derived `status` and `effective_offset` (the marker position when
/// addressed, else the stored anchor offset). The stored row is never mutated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReanchoredComment {
    #[serde(flatten)]
    pub comment: Comment,
    pub status: CommentStatus,
    /// Effective anchor offset in the viewed version (i64 to match the wire
    /// type of `anchor_offset`; `None` for direction comments).
    pub effective_offset: Option<i64>,
}

/// Project the stored comments onto `version_markdown`: a comment whose id is
/// marked `addressed` in the version is re-anchored to the marker's offset and
/// marked `Addressed`; every other inline/direction comment carries over as
/// `Open` at its stored offset. Pure — derives from inputs, persists nothing.
///
/// NOTE: the no-marker default here (no marker => `Open` at stored offset) is
/// mirrored by the `useComments` frontend fallback for the no-version-body
/// case; the two must change in lockstep (vet F3). This Rust projector is the
/// single authoritative path whenever a version body is present (the only path
/// that ever yields `Addressed`).
///
/// Order: by effective offset (so re-anchored comments follow the new text),
/// then by `created_at` as a stable tiebreak, mirroring the store's
/// `created_at, anchor_offset` list order for carried-over comments.
pub fn reanchor_comments(comments: &[Comment], version_markdown: &str) -> Vec<ReanchoredComment> {
    let markers = parse_addressed_markers(version_markdown);
    let mut out: Vec<ReanchoredComment> = comments
        .iter()
        .map(|c| {
            let marker = markers.iter().find(|m| m.comment_id == c.id);
            match (marker, c.kind) {
                // Direction comments have no inline anchor; never re-anchored.
                (Some(m), CommentKind::Inline) => ReanchoredComment {
                    comment: c.clone(),
                    status: CommentStatus::Addressed,
                    effective_offset: Some(m.offset as i64),
                },
                (Some(_), CommentKind::Direction) | (None, _) => ReanchoredComment {
                    comment: c.clone(),
                    status: CommentStatus::Open,
                    effective_offset: c.anchor_offset,
                },
            }
        })
        .collect();
    out.sort_by(|a, b| {
        a.effective_offset
            .cmp(&b.effective_offset)
            .then(a.comment.created_at.cmp(&b.comment.created_at))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_marker_with_offset() {
        let md = "intro\n<!-- addressed: c1 -->\nrest";
        let m = parse_addressed_markers(md);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].comment_id, "c1");
        assert_eq!(m[0].offset, md.find("<!--").unwrap());
    }

    #[test]
    fn tolerates_whitespace_variants() {
        let md = "<!--addressed:c1--> x <!-- addressed:   c2   -->";
        let m = parse_addressed_markers(md);
        assert_eq!(
            m.iter().map(|x| x.comment_id.as_str()).collect::<Vec<_>>(),
            vec!["c1", "c2"]
        );
    }

    #[test]
    fn ignores_non_addressed_comments_and_empty_ids() {
        let md = "<!-- TODO: nope --> <!-- addressed: --> <!-- addressed: real -->";
        let m = parse_addressed_markers(md);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].comment_id, "real");
    }

    #[test]
    fn handles_no_markers_and_unterminated_comment() {
        assert!(parse_addressed_markers("plain text").is_empty());
        assert!(parse_addressed_markers("<!-- addressed: c1 (never closed").is_empty());
    }

    fn cmt(id: &str, offset: Option<i64>, kind: CommentKind, created: i64) -> Comment {
        Comment {
            id: id.into(),
            task_id: "T-1".into(),
            artifact_path: "artifacts/specs/T-1-v1.md".into(),
            anchor_text: Some("quote".into()),
            anchor_offset: offset,
            note: "note".into(),
            kind,
            created_at: created,
        }
    }

    #[test]
    fn addressed_comment_is_reanchored_to_marker_offset() {
        let comments = vec![cmt("c1", Some(5), CommentKind::Inline, 100)];
        let md = "header\n\n<!-- addressed: c1 --> resolved here";
        let out = reanchor_comments(&comments, md);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, CommentStatus::Addressed);
        assert_eq!(out[0].effective_offset, Some(md.find("<!--").unwrap() as i64));
    }

    #[test]
    fn unaddressed_comment_carries_over_at_stored_offset() {
        let comments = vec![cmt("c1", Some(42), CommentKind::Inline, 100)];
        let out = reanchor_comments(&comments, "no markers here");
        assert_eq!(out[0].status, CommentStatus::Open);
        assert_eq!(out[0].effective_offset, Some(42));
    }

    #[test]
    fn direction_comment_never_reanchored_even_if_marked() {
        let comments = vec![cmt("c1", None, CommentKind::Direction, 100)];
        let md = "<!-- addressed: c1 -->";
        let out = reanchor_comments(&comments, md);
        assert_eq!(out[0].status, CommentStatus::Open);
        assert_eq!(out[0].effective_offset, None);
    }

    #[test]
    fn mixed_set_orders_by_effective_offset() {
        let comments = vec![
            cmt("c1", Some(50), CommentKind::Inline, 100), // unaddressed, stays at 50
            cmt("c2", Some(99), CommentKind::Inline, 101), // addressed -> marker offset 0
        ];
        let md = "<!-- addressed: c2 -->\nlater text past offset fifty.....................";
        let out = reanchor_comments(&comments, md);
        // c2 re-anchored to offset 0 sorts before c1 at 50.
        assert_eq!(out[0].comment.id, "c2");
        assert_eq!(out[0].status, CommentStatus::Addressed);
        assert_eq!(out[1].comment.id, "c1");
        assert_eq!(out[1].status, CommentStatus::Open);
    }
}
