//! The FanOutGroup aggregate (Runtime; root `group_id`). Owns the named
//! invariant: *"exactly one continuation task is emitted when all lanes of a
//! fan-out group have settled, and only once."* (vet F1). This module is the
//! PURE core — the expected lanes, each lane's verdict, the completed flag, and
//! the aggregation rule (all-approve ⇒ downstream, else ⇒ needs-human). The
//! atomic single-writer persistence guard lives in fanout_store.rs.

use agent_bus_core::Verdict;
use serde::{Deserialize, Serialize};

/// One lane's settled verdict within a group. A lane reaches the barrier only by
/// approving into its join (then `Approve`), or by routing to needs-human while
/// carrying a group_id (then `Reject`) — see Decision D5. Revise never reaches
/// the barrier (it loops within the lane).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneVerdict {
    pub lane: String,
    pub verdict: Verdict,
}

/// The fan-out group aggregate root. The consistency boundary for the barrier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanOutGroup {
    pub id: String,
    pub pipeline: String,
    pub join_target: String,
    pub downstream: String,
    pub expected_lanes: Vec<String>,
    pub completed: bool,
}

/// Where the single continuation past the barrier goes once all lanes settle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Continuation {
    /// All lanes approved ⇒ proceed to the join's downstream node.
    Downstream(String),
    /// Any lane rejected/exhausted ⇒ escalate the joined task.
    NeedsHuman,
}

impl FanOutGroup {
    /// True once every expected lane has a recorded verdict.
    pub fn all_lanes_settled(&self, recorded: &[LaneVerdict]) -> bool {
        self.expected_lanes
            .iter()
            .all(|lane| recorded.iter().any(|r| &r.lane == lane))
    }

    /// The continuation given all lanes' verdicts: all-approve ⇒ downstream,
    /// else ⇒ needs-human (the all-must-approve-else-needs-human rule).
    pub fn continuation(&self, recorded: &[LaneVerdict]) -> Continuation {
        let all_approve = recorded.iter().all(|r| r.verdict == Verdict::Approve);
        if all_approve && self.all_lanes_settled(recorded) {
            Continuation::Downstream(self.downstream.clone())
        } else {
            Continuation::NeedsHuman
        }
    }

    /// The continuation when the early-cancel policy (P2) fires: a lane has
    /// failed, so the group resolves to needs-human WITHOUT waiting for the other
    /// lanes. This is the aggregate root's early-cancel rule — the store asks for
    /// it after winning the completes-once guard, mirroring how the full barrier
    /// asks `continuation()`. Keeping it here (not hardcoded in the store) means a
    /// future divergent join policy has one home (vet F1).
    pub fn early_cancel_continuation(&self) -> Continuation {
        Continuation::NeedsHuman
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> FanOutGroup {
        FanOutGroup {
            id: "G-1".into(),
            pipeline: "pipe".into(),
            join_target: "join-1".into(),
            downstream: "after".into(),
            expected_lanes: vec!["lane-a".into(), "lane-b".into()],
            completed: false,
        }
    }

    #[test]
    fn not_all_settled_until_every_lane_has_a_verdict() {
        let g = group();
        let one = vec![LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve }];
        assert!(!g.all_lanes_settled(&one));
        let both = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Approve },
        ];
        assert!(g.all_lanes_settled(&both));
    }

    #[test]
    fn all_approve_continues_to_downstream() {
        let g = group();
        let settled = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Approve },
        ];
        assert_eq!(g.continuation(&settled), Continuation::Downstream("after".into()));
    }

    #[test]
    fn early_cancel_continuation_is_needs_human() {
        // The FanOutGroup owns the early-cancel aggregation rule: a failed lane
        // forces needs-human, regardless of the other (unsettled) lanes.
        let g = group();
        assert_eq!(g.early_cancel_continuation(), Continuation::NeedsHuman);
    }

    #[test]
    fn any_reject_continues_to_needs_human() {
        let g = group();
        let settled = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Reject },
        ];
        assert_eq!(g.continuation(&settled), Continuation::NeedsHuman);
    }
}
