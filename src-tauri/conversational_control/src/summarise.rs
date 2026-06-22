//! 24h-summarisation policy (D5; spec conversations-table comment). On launch, a
//! conversation whose last message is >24h old gets its turns folded into
//! summary_of_prior_sessions and a fresh session begins. The summary BODY is a
//! deterministic, model-free digest in v1 (count + first/last user lines); v1.1
//! may swap the body for a model-written summary behind this same signature.

use crate::conversation::Conversation;
use crate::turn::{Role, Turn};

pub const TWENTY_FOUR_HOURS_SECS: i64 = 86_400;

/// True when the conversation should be summarised on launch.
pub fn should_summarise(last_message_at: i64, now: i64) -> bool {
    now - last_message_at > TWENTY_FOUR_HOURS_SECS
}

/// Build the new summary text from the aging turns + any prior summary.
pub fn summarise_turns(turns: &[Turn], prior: Option<&str>) -> String {
    let user_lines: Vec<&str> = turns
        .iter()
        .filter(|t| t.role == Role::User)
        .map(|t| t.text.as_str())
        .collect();
    let mut body = String::new();
    if let Some(p) = prior {
        if !p.is_empty() {
            body.push_str(p);
            body.push_str("\n");
        }
    }
    body.push_str(&format!(
        "Prior session: {} turns ({} from you).",
        turns.len(),
        user_lines.len()
    ));
    if let Some(first) = user_lines.first() {
        body.push_str(&format!(" First: {first}."));
    }
    if user_lines.len() > 1 {
        if let Some(last) = user_lines.last() {
            body.push_str(&format!(" Last: {last}."));
        }
    }
    body
}

/// Apply the policy: if aged, fold turns into the summary and start a fresh
/// session (new session_id, started_at = now, cleared turns). Mutates in place;
/// returns true if it summarised.
pub fn summarise_on_launch(c: &mut Conversation, new_session_id: impl Into<String>, now: i64) -> bool {
    if c.turns.is_empty() || !should_summarise(c.last_message_at, now) {
        return false;
    }
    let summary = summarise_turns(&c.turns, c.summary_of_prior_sessions.as_deref());
    c.summary_of_prior_sessions = Some(summary);
    c.turns.clear();
    c.session_id = new_session_id.into();
    c.started_at = now;
    c.last_message_at = now;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_summarise_only_past_24h() {
        let now = 1_000_000;
        assert!(!should_summarise(now - 10, now));
        assert!(!should_summarise(now - TWENTY_FOUR_HOURS_SECS, now)); // exactly 24h: not yet
        assert!(should_summarise(now - TWENTY_FOUR_HOURS_SECS - 1, now));
    }

    #[test]
    fn summarise_turns_digests_user_lines() {
        let turns = vec![
            Turn::user("/inject one", 0),
            Turn::assistant("Done.", vec![], 1),
            Turn::user("/approve T-1", 2),
            Turn::assistant("Done.", vec![], 3),
        ];
        let s = summarise_turns(&turns, None);
        assert!(s.contains("4 turns"));
        assert!(s.contains("2 from you"));
        assert!(s.contains("First: /inject one"));
        assert!(s.contains("Last: /approve T-1"));
    }

    #[test]
    fn summarise_turns_appends_to_prior_summary() {
        let s = summarise_turns(&[Turn::user("hi", 0)], Some("Earlier: setup done."));
        assert!(s.starts_with("Earlier: setup done."));
        assert!(s.contains("Prior session: 1 turns"));
    }

    #[test]
    fn summarise_on_launch_folds_and_resets_when_aged() {
        let mut c = Conversation::new("p", "old-sess", 0);
        c.append(Turn::user("/inject x", 10)).unwrap();
        c.append(Turn::assistant("Done.", vec![], 11)).unwrap();
        let now = 11 + TWENTY_FOUR_HOURS_SECS + 5;
        let did = summarise_on_launch(&mut c, "new-sess", now);
        assert!(did);
        assert!(c.turns.is_empty());
        assert_eq!(c.session_id, "new-sess");
        assert_eq!(c.started_at, now);
        assert!(c.summary_of_prior_sessions.unwrap().contains("Prior session"));
    }

    #[test]
    fn summarise_on_launch_is_noop_when_recent() {
        let mut c = Conversation::new("p", "sess", 0);
        c.append(Turn::user("/inject x", 10)).unwrap();
        let did = summarise_on_launch(&mut c, "new-sess", 20);
        assert!(!did);
        assert_eq!(c.session_id, "sess");
        assert_eq!(c.turns.len(), 1);
    }
}
