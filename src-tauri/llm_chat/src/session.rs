//! The session map — the F3 seal. Maps the caller's stable `dialogue_id` to the
//! claude session id captured from stream-json. First turn for a dialogue has no
//! mapping (so no --resume); after a successful turn the session is recorded, so
//! the next turn resumes it. A lost session is cleared so the next turn starts
//! fresh. None of this is observable outside `llm_chat`.

use std::collections::HashMap;
use std::sync::Mutex;

/// In-process map from a caller's `dialogue_id` to the claude session id. Behind
/// a Mutex so a `&self` ChatRunner (held as Arc) can mutate it across turns.
#[derive(Debug, Default)]
pub struct SessionMap {
    inner: Mutex<HashMap<String, String>>,
}

impl SessionMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// The claude session for this dialogue, if one is known (=> pass --resume).
    pub fn get(&self, dialogue_id: &str) -> Option<String> {
        self.inner.lock().unwrap().get(dialogue_id).cloned()
    }

    /// Record (or overwrite) the claude session for this dialogue after a
    /// successful turn.
    pub fn record(&self, dialogue_id: &str, session_id: &str) {
        self.inner
            .lock()
            .unwrap()
            .insert(dialogue_id.to_string(), session_id.to_string());
    }

    /// Forget the session for this dialogue (a lost/dead session) so the next
    /// turn starts fresh. The caller's dialogue_id is untouched.
    pub fn clear(&self, dialogue_id: &str) {
        self.inner.lock().unwrap().remove(dialogue_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_lookup_for_a_dialogue_is_none() {
        let map = SessionMap::new();
        assert_eq!(map.get("proj-1"), None);
    }

    #[test]
    fn recorded_session_is_returned_on_the_next_turn() {
        let map = SessionMap::new();
        map.record("proj-1", "sess-abc");
        assert_eq!(map.get("proj-1").as_deref(), Some("sess-abc"));
        // a different dialogue is independent
        assert_eq!(map.get("proj-2"), None);
    }

    #[test]
    fn record_overwrites_a_rotated_session_for_the_same_dialogue() {
        let map = SessionMap::new();
        map.record("proj-1", "sess-old");
        map.record("proj-1", "sess-new");
        assert_eq!(map.get("proj-1").as_deref(), Some("sess-new"));
    }

    #[test]
    fn clear_forces_a_fresh_session_next_turn() {
        let map = SessionMap::new();
        map.record("proj-1", "sess-dead");
        map.clear("proj-1");
        assert_eq!(map.get("proj-1"), None);
    }
}
