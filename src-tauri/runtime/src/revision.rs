//! The revise-bundle CONSUMER (Plan 4 vet F1). Runtime owns the invocation, so
//! reading Review's persisted comments and composing them into the re-claimed
//! task's user message is a Runtime change — but Runtime must NOT depend on the
//! `review` crate (that would cross context lines + risk a cycle). So this is a
//! Runtime-LOCAL trait seam returning plain strings; the concrete reader that
//! queries the `comments` table is wired at the composition root (the only
//! module importing both contexts). Mirrors the UsageSink seam (Plan 5).

use async_trait::async_trait;

/// One revise note, flattened to strings so no Review type crosses the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionNote {
    /// The quoted span (None for the overall-direction note).
    pub anchor_text: Option<String>,
    pub note: String,
    /// "inline" or "direction".
    pub kind: String,
}

/// The persisted revise bundle for a task, newest-version first by the store's
/// ordering. Empty when the task has no comments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RevisionBundle {
    pub notes: Vec<RevisionNote>,
}

/// Loads a task's revise bundle. Implemented at the root over the comments table.
#[async_trait]
pub trait RevisionBundleReader: Send + Sync {
    async fn load(&self, task_id: &str) -> RevisionBundle;
}

/// Compose the user message for an invocation. On a fresh run (no reader, or an
/// empty bundle) returns the topic unchanged. On a re-claim with a bundle,
/// appends a deterministic, prompt-shaped revision request. No em dashes in the
/// emitted text (DESIGN.md copy rule): uses ":" and "->".
pub async fn compose_invocation_message(
    topic: &str,
    attempts: u32,
    reader: Option<&dyn RevisionBundleReader>,
    task_id: &str,
) -> String {
    // D2: only a re-claim (attempts > 1) carries a bundle.
    if attempts <= 1 {
        return topic.to_string();
    }
    let Some(reader) = reader else { return topic.to_string() };
    let bundle = reader.load(task_id).await;
    if bundle.notes.is_empty() {
        return topic.to_string();
    }

    let mut out = String::from(topic);
    out.push_str(&format!("\n\n--- REVISION REQUEST (attempt {attempts}) ---\n"));

    let direction: Vec<&RevisionNote> =
        bundle.notes.iter().filter(|n| n.kind == "direction").collect();
    let inline: Vec<&RevisionNote> =
        bundle.notes.iter().filter(|n| n.kind == "inline").collect();

    if let Some(d) = direction.first() {
        out.push_str("\nOverall direction:\n");
        out.push_str(&d.note);
        out.push('\n');
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
    async fn load(&self, _task_id: &str) -> RevisionBundle {
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
        let msg = compose_invocation_message("the topic", 1, None, "T-1").await;
        assert_eq!(msg, "the topic");
    }

    #[tokio::test]
    async fn reclaim_with_no_reader_returns_topic_only() {
        let msg = compose_invocation_message("the topic", 2, None, "T-1").await;
        assert_eq!(msg, "the topic");
    }

    #[tokio::test]
    async fn reclaim_with_empty_bundle_returns_topic_only() {
        let reader = FakeRevisionReader { bundle: RevisionBundle::default() };
        let msg = compose_invocation_message("the topic", 2, Some(&reader), "T-1").await;
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
        let msg = compose_invocation_message("Scheduling bulk-write", 2, Some(&reader), "T-40").await;
        assert!(msg.starts_with("Scheduling bulk-write"));
        assert!(msg.contains("REVISION REQUEST (attempt 2)"));
        assert!(msg.contains("Overall direction:"));
        assert!(msg.contains("rewrite tasks 1 and 2"));
        assert!(msg.contains("Inline comments:"));
        assert!(msg.contains("1. \"idempotency key\" -> per-row, not per-batch"));
        assert!(!msg.contains(" — "), "no em dashes in composed text");
    }
}
