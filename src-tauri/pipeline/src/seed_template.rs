//! Bundled template **seeds** for the wizard's Design Session (DOMAIN.md →
//! Pipeline Authoring, "Template (seed)"). A seed is a `DraftPipeline` — the
//! editable, not-yet-valid draft the wizard owns — NOT the validated `Pipeline`
//! aggregate, and NOT the dropped instantiate-on-create path (sub-project 3).
//! `list_seed_templates()` returns light summaries for the kickoff picker;
//! `seed_template(id)` returns a populated draft that the wizard then refines and
//! creates through the normal hard-validate-at-create path. Prompt text is held
//! inline as `prompt_body` (like every draft), so a seeded draft is immediately a
//! complete, creatable draft.
//!
//! REGISTRY INVARIANT (vet F2): every bundled seed is a complete, creatable
//! `DraftPipeline` — non-empty prompt bodies on every team, all routes resolve to
//! known nodes, and it passes hard validation after `to_pipeline()`. The tests
//! enforce this named contract for the DDD seed.

use crate::draft::{DraftPipeline, DraftTeam};
use crate::model::{Escalation, Gate, Role, Routes, Store, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};

/// A light summary of a bundled seed template, for the kickoff picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// The catalog of bundled seed templates (id/name/description only).
pub fn seed_templates() -> Vec<SeedTemplate> {
    vec![SeedTemplate {
        id: "ddd-spec-plan-impl".to_string(),
        name: "DDD: Spec → Plan → Implement".to_string(),
        description: "Domain-driven design pipeline: research → spec → spec-review \
            → plan → plan-review → implement → code-review → hand off to human, \
            with a spec-approval human gate."
            .to_string(),
    }]
}

/// Return a populated `DraftPipeline` seed for the given template id, or `None`
/// for an unknown id (the OHS command maps `None` to an error).
pub fn seed_template(id: &str) -> Option<DraftPipeline> {
    match id {
        "ddd-spec-plan-impl" => Some(ddd_seed()),
        _ => None,
    }
}

/// Build a producer `DraftTeam` with an inline prompt body, an on_approve route,
/// and a bounded store capacity. Other routes default to None.
fn producer(id: &str, name: &str, prompt: &str, on_approve: &str, capacity: u32) -> DraftTeam {
    let mut t = DraftTeam::new(id, name);
    t.prompt_body = prompt.to_string();
    t.role = Role::Producer;
    t.store = Store { capacity };
    t.outputs = Routes {
        on_approve: Some(on_approve.to_string()),
        on_revise: None,
        on_reject: None,
    };
    t
}

/// Build a reviewer `DraftTeam` (role set EXPLICITLY — never inferred from the
/// name): all three routes wired — approve→downstream, revise→its writer,
/// decline→needs-human — so the team is well-connected by construction (G3/G15).
fn reviewer(id: &str, name: &str, prompt: &str, on_approve: &str, revise_to: &str, capacity: u32) -> DraftTeam {
    let mut t = DraftTeam::new(id, name);
    t.prompt_body = prompt.to_string();
    t.role = Role::Reviewer;
    t.store = Store { capacity };
    t.outputs = Routes {
        on_approve: Some(on_approve.to_string()),
        on_revise: Some(revise_to.to_string()),
        on_reject: Some("needs-human".to_string()),
    };
    t
}

/// The canonical DDD seed (G15): research → spec → spec-review → plan →
/// plan-review → implement → code-review → hand off to human. A complete,
/// creatable `DraftPipeline` — explicit reviewer roles with revise/decline routes,
/// the spec-approval human gate, sensible bounded-store capacities, inline prompt
/// bodies — that passes hard validation after `to_pipeline()` on the current
/// role/store/gate model. `needs-human` is the terminal hand-off-to-human node.
fn ddd_seed() -> DraftPipeline {
    let mut d = DraftPipeline::empty();
    d.id = "ddd-spec-plan-impl".to_string();
    d.name = "DDD: Spec → Plan → Implement".to_string();
    d.description =
        "Research → spec → spec-review → plan → plan-review → implement → code-review \
         → hand off to human. A spec-approval human gate; implementers work in a local \
         git worktree (no push); reviewers use /ddd-council vet."
            .to_string();
    d.schema_version = SCHEMA_VERSION;

    d.teams = vec![
        // The entry/source — scans the target repo for work, no input store concern.
        producer(
            "research",
            "Research",
            "You investigate the target repository and any existing artifacts to find \
             concrete, well-scoped units of work for this delivery. For each candidate, \
             record its location, why it is a candidate, and the proposed change. Aim for \
             whole-scope coverage. Read-only — produce a candidate analysis; do not modify \
             code. Emit one work-item per candidate.",
            "spec-writers",
            8,
        ),
        producer(
            "spec-writers",
            "Spec Writers",
            "For each candidate, write a change specification. Invoke the superpowers \
             brainstorming skill with recommended defaults and no operator intervention \
             (never pause for questions), then write the spec: the target, the proposed \
             change, the methods and call-sites affected, and explicit acceptance criteria. \
             Produce a Markdown artifact; do not implement.",
            "spec-reviewers",
            6,
        ),
        // Spec review — approve routes to the spec-approval human gate.
        reviewer(
            "spec-reviewers",
            "Spec Reviewers",
            "You review each specification for viability and soundness — scope, \
             testability, and whether the approach is correct. Approve to send it to the \
             human sign-off gate, request revision back to the spec writers, or decline \
             (to needs-human) if it is unworkable.",
            "gate-spec",
            "spec-writers",
            4,
        ),
        producer(
            "plan-writers",
            "Plan Writers",
            "You turn each approved specification into a concrete implementation plan. \
             Invoke the superpowers writing-plans skill (TDD, bite-sized tasks, exact file \
             paths, frequent commits). Produce a Markdown artifact; do not implement.",
            "plan-reviewers",
            6,
        ),
        reviewer(
            "plan-reviewers",
            "Plan Reviewers",
            "You vet each plan with /ddd-council vet (DDD soundness — boundaries, \
             aggregates, the right seams) plus a general implementation review (sequencing, \
             testability, completeness). Approve sound plans to the implementers, request \
             revision back to the plan writers, or decline to needs-human.",
            "implementers",
            "plan-writers",
            4,
        ),
        producer(
            "implementers",
            "Implementers",
            "You implement the approved plan in a SELF-CONTAINED git worktree of the target \
             repository, using the superpowers subagent-driven-development skill (TDD, with \
             a two-stage review per task). NEVER push, fetch, or otherwise touch any remote \
             — everything stays local. Commit the changes in the worktree and hand the diff \
             to code review; do not merge.",
            "code-reviewers",
            6,
        ),
        // Code review — approve hands the finished work off to a human (needs-human).
        reviewer(
            "code-reviewers",
            "Code Reviewers",
            "You review the implemented changes against their spec and plan for conformance, \
             quality, and test coverage. Approve to hand the finished work off to a human, \
             request revision (send back) to the implementers, or decline.",
            "needs-human",
            "implementers",
            4,
        ),
    ];

    // Producers that should be able to send work back / escalate too. (Reviewers
    // already carry all three via the `reviewer` helper.)
    for t in d.teams.iter_mut() {
        match t.id.as_str() {
            "spec-writers" => {
                t.outputs.on_revise = Some("research".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            "plan-writers" => {
                t.outputs.on_revise = Some("spec-writers".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            "implementers" => {
                t.outputs.on_revise = Some("plan-writers".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            _ => {}
        }
    }

    // The spec-approval human gate (G15): approved specs route on to the planners.
    d.gates = vec![Gate {
        id: "gate-spec".into(),
        label: "Spec Approval (human)".into(),
        downstream: "plan-writers".into(),
    }];

    // The terminal hand-off-to-human node (declines + the final code-review approve).
    d.escalations = vec![Escalation {
        id: "needs-human".into(),
        triggers: vec!["attempts >= 3".into(), "verdict == reject".into()],
    }];

    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::best_effort_validate;

    #[test]
    fn catalog_contains_the_ddd_seed() {
        let t = seed_templates();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "ddd-spec-plan-impl");
        assert!(!t[0].name.is_empty());
        assert!(!t[0].description.is_empty());
    }

    #[test]
    fn unknown_id_returns_none() {
        assert!(seed_template("nope").is_none());
    }

    #[test]
    fn ddd_seed_is_the_full_eight_stage_flow_with_the_spec_gate() {
        // G15: research → spec → spec-review → plan → plan-review → implement →
        // code-review (7 teams), one spec-approval human gate, one needs-human
        // (hand-off-to-human) terminal escalation.
        let d = seed_template("ddd-spec-plan-impl").unwrap();
        assert_eq!(d.teams.len(), 7);
        let ids: Vec<&str> = d.teams.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "research",
                "spec-writers",
                "spec-reviewers",
                "plan-writers",
                "plan-reviewers",
                "implementers",
                "code-reviewers",
            ]
        );
        assert_eq!(d.gates.len(), 1);
        assert_eq!(d.gates[0].id, "gate-spec");
        assert_eq!(d.escalations.len(), 1);
        assert_eq!(d.escalations[0].id, "needs-human");
        assert_eq!(d.id, "ddd-spec-plan-impl");
        assert_eq!(d.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn ddd_seed_reviewers_have_explicit_roles_and_revise_decline_routes() {
        // G15/G3: every review stage sets role=Reviewer EXPLICITLY (not inferred)
        // and carries approve + revise→its writer + decline→needs-human.
        let d = seed_template("ddd-spec-plan-impl").unwrap();
        for id in ["spec-reviewers", "plan-reviewers", "code-reviewers"] {
            let r = d.teams.iter().find(|t| t.id == id).unwrap();
            assert_eq!(r.role, Role::Reviewer, "{id} must be a reviewer");
            assert!(r.outputs.on_approve.is_some(), "{id} missing approve route");
            assert!(r.outputs.on_revise.is_some(), "{id} missing revise route");
            assert_eq!(r.outputs.on_reject.as_deref(), Some("needs-human"), "{id} must decline to needs-human");
        }
        // The final code review hands approved work off to a human.
        let code = d.teams.iter().find(|t| t.id == "code-reviewers").unwrap();
        assert_eq!(code.outputs.on_approve.as_deref(), Some("needs-human"));
    }

    #[test]
    fn ddd_seed_stores_have_nonzero_capacity() {
        let d = seed_template("ddd-spec-plan-impl").unwrap();
        for t in &d.teams {
            assert!(t.store.capacity >= 1, "team {} has a zero-capacity store", t.id);
        }
    }

    #[test]
    fn ddd_seed_teams_all_have_inline_prompt_bodies() {
        // DD4: the seed is immediately complete — no empty prompts.
        let d = seed_template("ddd-spec-plan-impl").unwrap();
        for t in &d.teams {
            assert!(!t.prompt_body.trim().is_empty(), "team {} has no prompt body", t.id);
        }
    }

    #[test]
    fn ddd_seed_is_a_clean_best_effort_draft() {
        // Every route points at a known node (team/gate/escalation) and every
        // team has a prompt — best-effort validation is silent.
        let d = seed_template("ddd-spec-plan-impl").unwrap();
        assert_eq!(best_effort_validate(&d), Vec::<String>::new());
    }

    #[test]
    fn ddd_seed_hard_validates_after_to_pipeline() {
        // The seed is a complete, creatable draft: to_pipeline() + hard validate
        // passes, so the normal create path accepts it unmodified.
        let d = seed_template("ddd-spec-plan-impl").unwrap();
        let p = d.to_pipeline();
        let resolved = crate::resolve::resolve_defaults(&p);
        assert_eq!(crate::validate::validate(&resolved), Ok(()));
    }
}
