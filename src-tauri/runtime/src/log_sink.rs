//! log_sink — the per-task live-log sink factory type (R4 streaming).
//!
//! Extracted from the (now-deleted) single-task `pool` module at the ④d cutover
//! cleanup: this is the one item from that module still consumed by live code —
//! the composition root (`app::lib::make_task_log_sink`) builds a factory and the
//! worker deps bundle (`app::pipeline_activator::WorkerDeps.log_sink`) carries it.

use runners::output::LogSink;

/// Builds a per-task display-only log sink. Given a task id, returns a `LogSink`
/// the runner forwards assistant prose to as the worker streams. Display-only:
/// the deltas never influence settle/route (R4). None = no streaming (the
/// runner's non-streaming `invoke` is used) — the no-op case for runtime-only
/// tests. (See DOMAIN.md → Runtime "Stream (live log)" / Runners "Log delta".)
pub type LogSinkFactory = dyn Fn(&str) -> LogSink + Send + Sync;
