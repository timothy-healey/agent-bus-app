//! Canonical Tauri event-name contract for the `emit(...)` side.
//!
//! IMPORTANT: Tauri 2 validates event names against a fixed charset —
//! alphanumeric plus `-`, `/`, `:`, `_`. A `.` is REJECTED at the `listen`
//! boundary on the frontend (silent unhandled rejection → no listener → the UI
//! never refreshes). These MUST stay byte-for-byte identical to the TS contract
//! in `src/ipc/events.ts`. The charset guard below mirrors `events.test.ts`.

/// A task aggregate changed (created/claimed/settled/routed). Board + list refetch.
pub const TASK_CHANGED: &str = "task-changed";
/// A run aggregate changed (started/completed). Board + run selector refetch (④e).
pub const RUN_CHANGED: &str = "run-changed";
/// Usage/brake state changed. Meter refetches.
pub const USAGE_CHANGED: &str = "usage-changed";
/// Display-only live-log fragment for one running task (R4).
pub const TASK_LOG: &str = "task-log";
/// Display-only streamed assistant prose fragment for the terminal (D8).
pub const CONVERSATION_DELTA: &str = "conversation-delta";
/// A generator (source) pass started or settled (LF31). Payload:
/// `{ run_id, stage, task_id, active }`. The frontend renders a transient
/// clickable source card while `active`.
pub const GENERATOR_STATUS: &str = "generator-status";

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for the dotted-event-name bug: every emitted name must be
    /// a Tauri-legal event name (alphanumeric, `-`, `/`, `:`, `_`). A `.` silently
    /// breaks every frontend subscription.
    #[test]
    fn all_event_names_are_tauri_legal() {
        for name in [TASK_CHANGED, RUN_CHANGED, USAGE_CHANGED, TASK_LOG, CONVERSATION_DELTA, GENERATOR_STATUS] {
            assert!(!name.is_empty(), "event name must be non-empty");
            assert!(
                name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '/' | ':' | '_')),
                "event name `{name}` contains a char outside Tauri's allowed set (notably `.`)"
            );
        }
    }
}
