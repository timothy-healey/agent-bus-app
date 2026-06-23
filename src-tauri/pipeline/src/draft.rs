//! DraftPipeline — an in-progress, NOT-yet-valid pipeline the wizard edits
//! (DOMAIN.md → Pipeline Authoring). Distinct from the validated `Pipeline`
//! aggregate (vet F2): a half-built draft cannot satisfy validate.rs, so the
//! wizard manipulates this looser structure. Best-effort validation surfaces
//! issues live; only a draft that passes HARD validation (validate::validate on
//! its to_pipeline()) becomes a `Pipeline`. Prompt text is held inline as
//! `prompt_body`; to_pipeline() converts it to a `prompts/<id>.md` path.

use crate::model::{Escalation, Fork, Join, Routes, RunnerConfig, Scope, Workers, SCHEMA_VERSION};
use agent_bus_core::{EffortMode, RunnerKind};
use serde::{Deserialize, Serialize};

/// A team as the wizard edits it. Same fields as `model::Team` except the prompt
/// is held inline as text (`prompt_body`), not a file path — the wizard edits
/// prompt *content*. `to_pipeline()` (Task 5) writes it to `prompts/<id>.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftTeam {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub prompt_body: String,
    pub runner: RunnerConfig,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub outputs: Routes,
    #[serde(default)]
    pub workers: Workers,
}

impl DraftTeam {
    /// A new team with smart defaults (claude-cli, standard effort, 1/1 workers,
    /// empty prompt + scope + routes). `kickoff_generate` overrides these.
    pub fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            prompt_body: String::new(),
            runner: RunnerConfig {
                kind: RunnerKind::ClaudeCli,
                model: "claude-opus-4-8".into(),
                effort: EffortMode::Standard,
                api_key_env: None,
            },
            scope: Scope::default(),
            outputs: Routes::default(),
            workers: Workers::default(),
        }
    }
}

/// The in-progress pipeline. Looser than `Pipeline`: ids may be blank, teams may
/// be empty, routes may dangle — none of that is an error here (that is what
/// best-effort validation reports). Only `to_pipeline()` + hard validate gates
/// the transition to a real `Pipeline`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftPipeline {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub teams: Vec<DraftTeam>,
    #[serde(default)]
    pub forks: Vec<Fork>,
    #[serde(default)]
    pub joins: Vec<Join>,
    #[serde(default)]
    pub escalations: Vec<Escalation>,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl DraftPipeline {
    /// A blank draft (before kickoff). schema_version defaults to the current
    /// version so a draft that grows fork/join lanes is already v2.
    pub fn empty() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            schema_version: SCHEMA_VERSION,
            teams: vec![],
            forks: vec![],
            joins: vec![],
            escalations: vec![],
        }
    }
}

/// One team in a teams slice (id + display name only; technical config keeps its
/// existing/default values across a merge).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceTeam {
    pub id: String,
    pub name: String,
}

/// Step 2 slice: the full team set (add/remove/rename). Known teams keep their
/// prompt body + runner/scope; new teams get defaults; dropped teams are removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamsSlice {
    pub teams: Vec<SliceTeam>,
}

/// Step 3 slice: one team's responsibility prompt text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSlice {
    pub team_id: String,
    pub prompt_body: String,
}

/// One team's routing edges in a wiring slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteEdge {
    pub team_id: String,
    #[serde(default)]
    pub on_approve: Option<String>,
    #[serde(default)]
    pub on_revise: Option<String>,
    #[serde(default)]
    pub on_reject: Option<String>,
}

/// Step 4 slice: the fork/join wiring + per-team routes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WiringSlice {
    #[serde(default)]
    pub routes: Vec<RouteEdge>,
    #[serde(default)]
    pub forks: Vec<Fork>,
    #[serde(default)]
    pub joins: Vec<Join>,
}

/// A structured slice the model emits (one per wizard step). Internally tagged on
/// `kind` so a single fenced ```json block round-trips. The backend is the trust
/// boundary: only this typed slice mutates the draft, never the model's prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Slice {
    Teams(TeamsSlice),
    Prompt(PromptSlice),
    Wiring(WiringSlice),
}

/// Apply a slice to the draft in place. Pure; tolerant (unknown team in a prompt
/// slice is a no-op, dropped teams are removed). This is the merge half of the
/// two-way binding.
pub fn apply_slice(draft: &mut DraftPipeline, slice: Slice) {
    match slice {
        Slice::Teams(s) => {
            let mut next: Vec<DraftTeam> = Vec::with_capacity(s.teams.len());
            for st in s.teams {
                // Preserve an existing team's full config across the rename/merge.
                if let Some(existing) = draft.teams.iter().find(|t| t.id == st.id) {
                    let mut kept = existing.clone();
                    kept.name = st.name;
                    next.push(kept);
                } else {
                    next.push(DraftTeam::new(&st.id, &st.name));
                }
            }
            draft.teams = next;
        }
        Slice::Prompt(s) => {
            if let Some(team) = draft.teams.iter_mut().find(|t| t.id == s.team_id) {
                team.prompt_body = s.prompt_body;
            }
        }
        Slice::Wiring(s) => {
            for edge in &s.routes {
                if let Some(team) = draft.teams.iter_mut().find(|t| t.id == edge.team_id) {
                    team.outputs = Routes {
                        on_approve: edge.on_approve.clone(),
                        on_revise: edge.on_revise.clone(),
                        on_reject: edge.on_reject.clone(),
                    };
                }
            }
            draft.forks = s.forks;
            draft.joins = s.joins;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_draft_has_no_teams_and_default_schema_version() {
        let d = DraftPipeline::empty();
        assert!(d.teams.is_empty());
        assert_eq!(d.schema_version, crate::model::SCHEMA_VERSION);
        assert!(d.forks.is_empty());
        assert!(d.joins.is_empty());
    }

    #[test]
    fn draft_round_trips_through_serde_json() {
        let mut d = DraftPipeline::empty();
        d.id = "pl-1".into();
        d.name = "P".into();
        d.teams.push(DraftTeam::new("research", "Research"));
        let s = serde_json::to_string(&d).unwrap();
        let back: DraftPipeline = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn draft_team_new_has_sane_defaults() {
        let t = DraftTeam::new("research", "Research");
        assert_eq!(t.id, "research");
        assert_eq!(t.name, "Research");
        assert_eq!(t.prompt_body, "");
        assert_eq!(t.runner.kind, agent_bus_core::RunnerKind::ClaudeCli);
        assert_eq!(t.runner.effort, agent_bus_core::EffortMode::Standard);
    }

    #[test]
    fn teams_slice_replaces_the_team_set_preserving_known_bodies() {
        let mut d = DraftPipeline::empty();
        // an existing team with a written prompt body
        let mut existing = DraftTeam::new("research", "Research");
        existing.prompt_body = "Investigate the codebase.".into();
        d.teams.push(existing);

        // the slice declares research (kept) + a new "writers" team
        let slice = Slice::Teams(TeamsSlice {
            teams: vec![
                SliceTeam { id: "research".into(), name: "Research".into() },
                SliceTeam { id: "writers".into(), name: "Writers".into() },
            ],
        });
        apply_slice(&mut d, slice);

        assert_eq!(d.teams.len(), 2);
        // the existing research team's prompt body is preserved across the merge
        let research = d.teams.iter().find(|t| t.id == "research").unwrap();
        assert_eq!(research.prompt_body, "Investigate the codebase.");
        // the new writers team exists with defaults
        assert!(d.teams.iter().any(|t| t.id == "writers"));
    }

    #[test]
    fn prompt_slice_sets_one_team_body() {
        let mut d = DraftPipeline::empty();
        d.teams.push(DraftTeam::new("research", "Research"));
        apply_slice(&mut d, Slice::Prompt(PromptSlice {
            team_id: "research".into(),
            prompt_body: "You investigate the target repo and write findings.".into(),
        }));
        assert_eq!(d.teams[0].prompt_body, "You investigate the target repo and write findings.");
    }

    #[test]
    fn prompt_slice_for_unknown_team_is_a_no_op() {
        let mut d = DraftPipeline::empty();
        d.teams.push(DraftTeam::new("research", "Research"));
        apply_slice(&mut d, Slice::Prompt(PromptSlice {
            team_id: "ghost".into(),
            prompt_body: "x".into(),
        }));
        assert_eq!(d.teams[0].prompt_body, "");
    }

    #[test]
    fn wiring_slice_replaces_routes_forks_and_joins() {
        let mut d = DraftPipeline::empty();
        d.teams.push(DraftTeam::new("entry", "Entry"));
        d.teams.push(DraftTeam::new("a", "A"));
        d.teams.push(DraftTeam::new("b", "B"));
        apply_slice(&mut d, Slice::Wiring(WiringSlice {
            routes: vec![
                RouteEdge { team_id: "entry".into(), on_approve: Some("fork-1".into()), on_revise: None, on_reject: None },
                RouteEdge { team_id: "a".into(), on_approve: Some("join-1".into()), on_revise: None, on_reject: None },
                RouteEdge { team_id: "b".into(), on_approve: Some("join-1".into()), on_revise: None, on_reject: None },
            ],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["a".into(), "b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["a".into(), "b".into()], downstream: "needs-human".into() }],
        }));
        assert_eq!(d.teams.iter().find(|t| t.id == "entry").unwrap().outputs.on_approve.as_deref(), Some("fork-1"));
        assert_eq!(d.forks.len(), 1);
        assert_eq!(d.joins[0].downstream, "needs-human");
    }
}
