/// Canonical Tauri event-name contract shared by every `listen`/`emit` pair.
///
/// IMPORTANT: Tauri 2 validates event names against a fixed charset —
/// alphanumeric plus `-`, `/`, `:`, `_`. A `.` is REJECTED at the `listen`
/// boundary (silent unhandled rejection → no listener → the UI never refreshes).
/// These MUST stay byte-for-byte identical to the Rust `emit(...)` names in
/// `src-tauri/app/src/{lib,pipeline_activator}.rs`. The matching guard lives in
/// `events.test.ts`.
export const EVENTS = {
  /// A task aggregate changed (created/claimed/settled/routed). Board + list refetch.
  taskChanged: "task-changed",
  /// Usage/brake state changed. Meter refetches.
  usageChanged: "usage-changed",
  /// Display-only live-log fragment for one running task (R4).
  taskLog: "task-log",
  /// Display-only streamed assistant prose fragment for the terminal (D8).
  conversationDelta: "conversation-delta",
} as const;

export type EventName = (typeof EVENTS)[keyof typeof EVENTS];
