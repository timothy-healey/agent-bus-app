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
}
