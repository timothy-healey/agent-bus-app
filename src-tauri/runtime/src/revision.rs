//! The revise-bundle seam (Plan 4 vet F1). Runtime owns the invocation, so
//! reading Review's persisted comments and composing them into the re-claimed
//! task's user message is a Runtime change — but Runtime must NOT depend on the
//! `review` crate (that would cross context lines + risk a cycle). So these are
//! Runtime-LOCAL trait seams over plain strings; the concrete reader and writer
//! over the `comments` table are wired at the composition root (the only module
//! importing both contexts). Mirrors the UsageSink seam (Plan 5).

use async_trait::async_trait;

/// One revise note, flattened to strings so no Review type crosses the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionNote {
    /// The quoted span (None for direction and review notes).
    pub anchor_text: Option<String>,
    pub note: String,
    /// "inline", "direction" or "review".
    pub kind: String,
}

/// The persisted revise bundle for an item: every comment on any task of its
/// lineage, oldest first. Empty when the item has no comments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RevisionBundle {
    pub notes: Vec<RevisionNote>,
}

/// Loads an item's revise bundle across its lineage: every task of run `run_id`
/// carrying `item_key`. A revise creates a new child task, so feedback attached
/// to an earlier task of the item must still reach the producer. Implemented at
/// the root over the comments table.
#[async_trait]
pub trait RevisionBundleReader: Send + Sync {
    async fn load(&self, run_id: &str, item_key: &str) -> RevisionBundle;
}

/// Stores a reviewer's verdict and reason as a `review` comment on the reviewed
/// task, anchored to the reviewed artifact. Best-effort: a failed write never
/// fails the invocation. Implemented at the root over the comments table.
#[async_trait]
pub trait ReviewCommentWriter: Send + Sync {
    async fn record_review(&self, task_id: &str, artifact_path: &str, note: &str);
}

/// The text of a review comment: `<team>: <verdict>. <reason>`. No em dash
/// (copy rule), since the note is shown in the UI and composed into prompts.
pub fn review_note(team: &str, verdict: agent_bus_core::Verdict, reason: &str) -> String {
    let v = match verdict {
        agent_bus_core::Verdict::Approve => "approve",
        agent_bus_core::Verdict::Revise => "revise",
        agent_bus_core::Verdict::Reject => "reject",
    };
    format!("{team}: {v}. {}", reason.trim())
}

/// Compose the user message for an invocation. On a fresh run (no reader, or an
/// empty bundle) returns the topic unchanged. On a re-claim with a bundle,
/// appends a deterministic, prompt-shaped revision request: the newest overall
/// direction, the reviewers' feedback, then the inline comments. No em dashes in
/// the emitted text (DESIGN.md copy rule): uses ":" and "->".
pub async fn compose_invocation_message(
    topic: &str,
    attempts: u32,
    reader: Option<&dyn RevisionBundleReader>,
    run_id: &str,
    item_key: &str,
) -> String {
    // D2: only a re-claim (attempts > 1) carries a bundle.
    if attempts <= 1 {
        return topic.to_string();
    }
    let Some(reader) = reader else { return topic.to_string() };
    let bundle = reader.load(run_id, item_key).await;
    if bundle.notes.is_empty() {
        return topic.to_string();
    }

    let mut out = String::from(topic);
    out.push_str(&format!("\n\n--- REVISION REQUEST (attempt {attempts}) ---\n"));

    let of = |kind: &str| -> Vec<&RevisionNote> { bundle.notes.iter().filter(|n| n.kind == kind).collect() };
    let direction = of("direction");
    let review = of("review");
    let inline = of("inline");

    if let Some(d) = direction.last() {
        out.push_str("\nOverall direction:\n");
        out.push_str(&d.note);
        out.push('\n');
    }
    if !review.is_empty() {
        out.push_str("\nReview feedback:\n");
        for r in &review {
            out.push_str(&format!("- {}\n", r.note));
        }
    }
    if !inline.is_empty() {
        out.push_str("\nInline comments:\n");
        for (i, c) in inline.iter().enumerate() {
            match &c.anchor_text {
                Some(a) => out.push_str(&format!("{}. \"{}\" -> {}\n", i + 1, a, c.note)),
                None => out.push_str(&format!("{}. {}\n", i + 1, c.note)),
            }
        }
    }
    out
}

#[cfg(test)]
pub struct FakeRevisionReader {
    pub bundle: RevisionBundle,
}

#[cfg(test)]
#[async_trait]
impl RevisionBundleReader for FakeRevisionReader {
    async fn load(&self, _run_id: &str, _item_key: &str) -> RevisionBundle {
        self.bundle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(kind: &str, anchor: Option<&str>, n: &str) -> RevisionNote {
        RevisionNote { anchor_text: anchor.map(String::from), note: n.into(), kind: kind.into() }
    }

    #[tokio::test]
    async fn fresh_run_returns_topic_only() {
        let msg = compose_invocation_message("the topic", 1, None, "R-1", "alpha").await;
        assert_eq!(msg, "the topic");
    }

    #[tokio::test]
    async fn reclaim_with_no_reader_returns_topic_only() {
        let msg = compose_invocation_message("the topic", 2, None, "R-1", "alpha").await;
        assert_eq!(msg, "the topic");
    }

    #[tokio::test]
    async fn reclaim_with_empty_bundle_returns_topic_only() {
        let reader = FakeRevisionReader { bundle: RevisionBundle::default() };
        let msg = compose_invocation_message("the topic", 2, Some(&reader), "R-1", "alpha").await;
        assert_eq!(msg, "the topic");
    }

    #[tokio::test]
    async fn reclaim_composes_direction_and_inline_comments() {
        let reader = FakeRevisionReader {
            bundle: RevisionBundle {
                notes: vec![
                    note("inline", Some("idempotency key"), "per-row, not per-batch"),
                    note("direction", None, "rewrite tasks 1 and 2 with per-row semantics"),
                ],
            },
        };
        let msg = compose_invocation_message("Orders bulk-write", 2, Some(&reader), "R-1", "orders").await;
        assert!(msg.starts_with("Orders bulk-write"));
        assert!(msg.contains("REVISION REQUEST (attempt 2)"));
        assert!(msg.contains("Overall direction:"));
        assert!(msg.contains("rewrite tasks 1 and 2"));
        assert!(msg.contains("Inline comments:"));
        assert!(msg.contains("1. \"idempotency key\" -> per-row, not per-batch"));
        assert!(!msg.contains(" — "), "no em dashes in composed text");
    }

    #[tokio::test]
    async fn reclaim_composes_review_feedback_labelled_with_the_reviewing_team() {
        let reader = FakeRevisionReader {
            bundle: RevisionBundle {
                notes: vec![
                    note("review", None, &review_note("spec-review", agent_bus_core::Verdict::Revise, "error handling is thin")),
                    note("inline", Some("retry"), "bound it"),
                ],
            },
        };
        let msg = compose_invocation_message("Alpha", 2, Some(&reader), "R-1", "alpha").await;
        let header = msg.find("--- REVISION REQUEST (attempt 2) ---").expect("the header");
        let review = msg.find("Review feedback:").expect("a review section");
        assert!(review > header, "review feedback sits under the header");
        assert!(msg.contains("- spec-review: revise. error handling is thin"));
        assert!(msg.contains("1. \"retry\" -> bound it"));
        assert!(!msg.contains('—'));
    }

    #[tokio::test]
    async fn the_newest_direction_note_wins() {
        let reader = FakeRevisionReader {
            bundle: RevisionBundle {
                notes: vec![note("direction", None, "old direction"), note("direction", None, "new direction")],
            },
        };
        let msg = compose_invocation_message("t", 3, Some(&reader), "R-1", "alpha").await;
        assert!(msg.contains("new direction"));
        assert!(!msg.contains("old direction"));
    }

    #[test]
    fn review_note_names_team_verdict_and_reason_without_an_em_dash() {
        assert_eq!(review_note("qa", agent_bus_core::Verdict::Reject, "unsafe"), "qa: reject. unsafe");
        assert_eq!(review_note("qa", agent_bus_core::Verdict::Approve, "fine"), "qa: approve. fine");
    }
}
