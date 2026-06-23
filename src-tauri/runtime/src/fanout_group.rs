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
    /// The enclosing group when this group is a nested fork inside a lane (P1).
    /// `None` at the root (a top-level fork). When `Some`, this group's
    /// completion settles the parent lane's verdict via the parent's barrier —
    /// the same completes-once guard, one level up (DD-P1-2).
    #[serde(default)]
    pub parent_group_id: Option<String>,
    /// The enclosing lane (entry team id) this group reports its resolution to.
    /// `Some` iff `parent_group_id` is `Some`.
    #[serde(default)]
    pub parent_lane: Option<String>,
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
    /// True when this group is a nested child (a fork inside a parent lane). A
    /// child settles its parent lane on completion rather than spawning the
    /// continuation directly (DD-P1-2).
    pub fn is_child(&self) -> bool {
        self.parent_group_id.is_some()
    }

    /// Map a resolved child group's continuation to the verdict its parent lane
    /// records at the parent barrier: the child resolving to its downstream means
    /// the lane "approved" into the parent join; needs-human means the lane
    /// failed. This is how a nested group's exactly-once resolution feeds the
    /// parent level's exactly-once barrier (DD-P1-2).
    pub fn parent_lane_verdict(cont: &Continuation) -> Verdict {
        match cont {
            Continuation::Downstream(_) => Verdict::Approve,
            Continuation::NeedsHuman => Verdict::Reject,
        }
    }

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

    /// The continuation under a quorum policy (P3): proceed to `downstream` as
    /// soon as `quorum` lanes approve (early-resolve on success); resolve to
    /// needs-human once reaching `quorum` is impossible (unsettled lanes plus
    /// approvals-so-far < quorum); otherwise `None` (keep waiting). The aggregate
    /// owns this rule — the store asks for it behind the completes-once guard.
    /// A reject/revise-cap lane simply counts as a non-approval; quorum governs
    /// success, so `cancel_on_reject` does not apply when a quorum is set.
    ///
    /// Precondition (vet F2): the caller passes a validated quorum in
    /// `1..=expected_lanes.len()` — `validate.rs` enforces this at load/save, so
    /// the aggregate never sees an out-of-range value. `saturating_sub` below
    /// keeps the arithmetic panic-free regardless.
    pub fn quorum_continuation(&self, recorded: &[LaneVerdict], quorum: u32) -> Option<Continuation> {
        let approvals = recorded.iter().filter(|r| r.verdict == Verdict::Approve).count() as u32;
        if approvals >= quorum {
            return Some(Continuation::Downstream(self.downstream.clone()));
        }
        let unsettled = (self.expected_lanes.len() as u32).saturating_sub(recorded.len() as u32);
        if approvals + unsettled < quorum {
            return Some(Continuation::NeedsHuman);
        }
        None
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
            parent_group_id: None,
            parent_lane: None,
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

    fn group3() -> FanOutGroup {
        FanOutGroup {
            id: "G-3".into(),
            pipeline: "pipe".into(),
            join_target: "join-1".into(),
            downstream: "after".into(),
            expected_lanes: vec!["lane-a".into(), "lane-b".into(), "lane-c".into()],
            completed: false,
            parent_group_id: None,
            parent_lane: None,
        }
    }

    #[test]
    fn quorum_reached_resolves_downstream_without_waiting() {
        let g = group3(); // 3 lanes, quorum 2
        let recorded = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Approve },
        ];
        assert_eq!(g.quorum_continuation(&recorded, 2), Some(Continuation::Downstream("after".into())));
    }

    #[test]
    fn quorum_not_yet_reached_keeps_waiting() {
        let g = group3(); // 3 lanes, quorum 2
        let recorded = vec![LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve }];
        // 1 approval, 2 unsettled: 1+2=3 >= 2, not yet 2 approvals => wait
        assert_eq!(g.quorum_continuation(&recorded, 2), None);
    }

    #[test]
    fn quorum_impossible_resolves_needs_human() {
        let g = group3(); // 3 lanes, quorum 2
        let recorded = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Reject },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Reject },
        ];
        // 0 approvals, 1 unsettled: 0+1=1 < 2 => impossible => needs-human
        assert_eq!(g.quorum_continuation(&recorded, 2), Some(Continuation::NeedsHuman));
    }

    #[test]
    fn quorum_one_resolves_on_first_approval() {
        let g = group3();
        let recorded = vec![LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve }];
        assert_eq!(g.quorum_continuation(&recorded, 1), Some(Continuation::Downstream("after".into())));
    }

    #[test]
    fn quorum_equal_to_lanes_is_all_must_approve() {
        let g = group(); // 2 lanes, quorum 2
        let one = vec![LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve }];
        assert_eq!(g.quorum_continuation(&one, 2), None); // still need lane-b
        let both = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Approve },
        ];
        assert_eq!(g.quorum_continuation(&both, 2), Some(Continuation::Downstream("after".into())));
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

    #[test]
    fn a_root_group_has_no_parent_and_a_child_group_does() {
        let root = group();
        assert!(!root.is_child());
        let child = FanOutGroup {
            id: "G-2".into(),
            pipeline: "pipe".into(),
            join_target: "join-2".into(),
            downstream: "after-2".into(),
            expected_lanes: vec!["x".into(), "y".into()],
            completed: false,
            parent_group_id: Some("G-1".into()),
            parent_lane: Some("lane-a".into()),
        };
        assert!(child.is_child());
        assert_eq!(child.parent_group_id.as_deref(), Some("G-1"));
        assert_eq!(child.parent_lane.as_deref(), Some("lane-a"));
    }

    #[test]
    fn parent_lane_verdict_maps_downstream_to_approve_and_needs_human_to_reject() {
        assert_eq!(
            FanOutGroup::parent_lane_verdict(&Continuation::Downstream("x".into())),
            Verdict::Approve
        );
        assert_eq!(
            FanOutGroup::parent_lane_verdict(&Continuation::NeedsHuman),
            Verdict::Reject
        );
    }
}
