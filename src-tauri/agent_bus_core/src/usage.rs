//! The Runners→Usage-Telemetry seam, expressed in the shared kernel so the
//! producer (Runtime, emitting on settle) and the consumer (Usage Telemetry's
//! store) never depend on each other's crate — only on this kernel module.
//! Context map: Runners → Usage Telemetry is Customer-Supplier; the value that
//! crosses is this `UsageEvent`; the consumer is any `UsageSink`.

use crate::{TaskId, TeamId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One unit of attributed usage produced by a completed invocation. Mirrors the
/// `worker_usage_log` columns. `task_id` is optional (a team can run without a
/// task in theory); `ts` is unix seconds, injected by the producer (D5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageEvent {
    pub ts: i64,
    pub team_id: TeamId,
    pub task_id: Option<TaskId>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

/// The consumer seam. Usage Telemetry implements this over `worker_usage_log`.
/// Runtime holds an `Arc<dyn UsageSink>` and calls `record` on settle. Object-
/// safe so it is held behind `Arc`. `record` is infallible from the producer's
/// view — a telemetry write failure must never fail a worker settle (it is
/// logged inside the impl), so the signature returns nothing.
pub trait UsageSink: Send + Sync {
    fn record(&self, event: UsageEvent);
}

/// A no-op sink, used by the composition root before a project is open and by
/// Runtime tests that don't care about usage.
pub struct NullUsageSink;
impl UsageSink for NullUsageSink {
    fn record(&self, _event: UsageEvent) {}
}

/// Convenience: a shared null sink.
pub fn null_sink() -> Arc<dyn UsageSink> {
    Arc::new(NullUsageSink)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_event_round_trips() {
        let e = UsageEvent {
            ts: 1000,
            team_id: TeamId("research".into()),
            task_id: Some(TaskId("T-1".into())),
            model: "claude-opus-4-7".into(),
            input_tokens: 100,
            output_tokens: 20,
            cache_creation: 5,
            cache_read: 3,
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: UsageEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn null_sink_swallows_records() {
        let sink = null_sink();
        sink.record(UsageEvent {
            ts: 0,
            team_id: TeamId("t".into()),
            task_id: None,
            model: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_creation: 0,
            cache_read: 0,
        });
    }
}
