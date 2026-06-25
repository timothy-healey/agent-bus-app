//! ProcessRegistry — the composition-root holder of live child process-GROUP
//! ids. The killable spawner registers a pgid on spawn and deregisters it on
//! completion; `kill_all` signals every registered group (SIGTERM, grace,
//! SIGKILL). Deliberately lives in the `app` crate: no Runner/ChatRunner ACL
//! trait ever gains process types — process control is a root concern.

use std::collections::HashSet;
use std::sync::Mutex;

/// Shared registry of live child process-group ids (pgid == leader pid because
/// the spawner sets `.process_group(0)`). Cloned behind an `Arc` at the root.
#[derive(Default)]
pub struct ProcessRegistry {
    /// The set of currently-live process-group ids.
    groups: Mutex<HashSet<i32>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            groups: Mutex::new(HashSet::new()),
        }
    }

    /// Record a live child process-group id (the spawner calls this right after
    /// spawn). Idempotent — re-registering the same pgid is a no-op.
    pub fn register(&self, pgid: i32) {
        self.groups.lock().unwrap().insert(pgid);
    }

    /// Drop a process-group id once its child has been waited on. A pgid that is
    /// not present is fine (best-effort).
    pub fn deregister(&self, pgid: i32) {
        self.groups.lock().unwrap().remove(&pgid);
    }

    /// Count of currently-registered groups (test/inspection helper).
    pub fn len(&self) -> usize {
        self.groups.lock().unwrap().len()
    }

    /// Whether the registry currently holds no live groups.
    pub fn is_empty(&self) -> bool {
        self.groups.lock().unwrap().is_empty()
    }

    /// Snapshot of the live pgids (used by `kill_all` so the kill loop does not
    /// hold the lock while sleeping).
    #[allow(dead_code)]
    fn snapshot(&self) -> Vec<i32> {
        self.groups.lock().unwrap().iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_then_deregister_leaves_the_set_empty() {
        let reg = ProcessRegistry::new();
        reg.register(1234);
        reg.register(5678);
        assert_eq!(reg.len(), 2);
        reg.deregister(1234);
        assert_eq!(reg.len(), 1);
        reg.deregister(5678);
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn deregister_unknown_pgid_is_a_noop() {
        let reg = ProcessRegistry::new();
        reg.deregister(9999);
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn register_is_idempotent_on_the_same_pgid() {
        let reg = ProcessRegistry::new();
        reg.register(42);
        reg.register(42);
        assert_eq!(reg.len(), 1);
    }
}
