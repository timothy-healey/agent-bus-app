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
use crate::model::{Escalation, Gate, Routes, SCHEMA_VERSION};
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
        name: "DDD Spec → Plan → Implement".to_string(),
        description: "Domain-driven design pipeline: research → spec → plan → \
            implement, with two human-review gates."
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

/// Build a `DraftTeam` with an inline prompt body + a single on_approve route.
/// Helper so the seed reads as a flow. Other routes default to None.
fn team(id: &str, name: &str, prompt: &str, on_approve: &str) -> DraftTeam {
    let mut t = DraftTeam::new(id, name);
    t.prompt_body = prompt.to_string();
    t.outputs = Routes {
        on_approve: Some(on_approve.to_string()),
        on_revise: None,
        on_reject: None,
    };
    t
}

/// The DDD spec→plan→implement seed, recovered from the historical bundled
/// template (00ef101^:.../ddd-spec-plan-impl.yaml) and reshaped as a draft:
/// inline prompt bodies (DD4), two human gates, one escalation. schema_version
/// is current (DD5).
fn ddd_seed() -> DraftPipeline {
    let mut d = DraftPipeline::empty();
    d.id = "ddd-spec-plan-impl".to_string();
    d.name = "DDD Spec → Plan → Implement".to_string();
    d.description = "Domain-driven design pipeline with two human gates.".to_string();
    d.schema_version = SCHEMA_VERSION;

    d.teams = vec![
        team(
            "research",
            "Research",
            "You investigate the target repository and existing artifacts, then \
             write a findings/critique analysis the spec writers will build on.",
            "spec-writers",
        ),
        team(
            "spec-writers",
            "Spec Writers",
            "You turn the research findings into a clear specification document \
             with explicit requirements and boundaries.",
            "spec-reviewers",
        ),
        team(
            "spec-reviewers",
            "Spec Reviewers",
            "You review the specification for completeness, soundness, and clarity, \
             then approve, request revision, or reject.",
            "gate-1-spec",
        ),
        team(
            "plan-writers",
            "Plan Writers",
            "You turn the approved specification into a concrete, step-by-step \
             implementation plan.",
            "plan-reviewers",
        ),
        team(
            "plan-reviewers",
            "Plan Reviewers",
            "You review the implementation plan against the spec, then approve, \
             request revision, or reject.",
            "gate-2-plan",
        ),
        team(
            "implementers",
            "Implementers",
            "You implement the approved plan in a worktree, committing the changes.",
            "done",
        ),
        {
            let mut done = DraftTeam::new("done", "Done");
            done.prompt_body =
                "You summarize the completed work. Terminal node — no further routing."
                    .to_string();
            done
        },
    ];

    // Revise/reject edges (recovered from the historical template).
    for t in d.teams.iter_mut() {
        match t.id.as_str() {
            "spec-writers" => {
                t.outputs.on_revise = Some("research".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            "spec-reviewers" => {
                t.outputs.on_revise = Some("spec-writers".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            "plan-writers" => {
                t.outputs.on_revise = Some("spec-writers".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            "plan-reviewers" => {
                t.outputs.on_revise = Some("plan-writers".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            "implementers" => {
                t.outputs.on_revise = Some("plan-writers".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            _ => {}
        }
    }

    d.gates = vec![
        Gate {
            id: "gate-1-spec".into(),
            label: "Gate 1 — Spec Approval".into(),
            downstream: "plan-writers".into(),
        },
        Gate {
            id: "gate-2-plan".into(),
            label: "Gate 2 — Plan Approval".into(),
            downstream: "implementers".into(),
        },
    ];

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
    fn ddd_seed_is_a_draft_with_seven_teams_and_two_gates() {
        let d = seed_template("ddd-spec-plan-impl").unwrap();
        assert_eq!(d.teams.len(), 7);
        assert_eq!(d.gates.len(), 2);
        assert_eq!(d.escalations.len(), 1);
        assert_eq!(d.id, "ddd-spec-plan-impl");
        assert_eq!(d.schema_version, SCHEMA_VERSION);
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
