//! The Pipeline aggregate and its node types. Mirrors the YAML schema in the
//! design spec (Data model → Pipeline definition). Pure data + serde; parsing
//! lives in parse.rs and invariant enforcement in validate.rs.

use agent_bus_core::{EffortMode, RunnerKind};
use serde::{Deserialize, Serialize};

/// The current pipeline schema version. Pipeline Authoring ↔ Runtime is a
/// Shared Kernel keyed on this number (context-map.md). Bumping it is a
/// breaking change reviewed by both contexts.
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pipeline {
    /// Root identity of the aggregate (context-map.md). Unique per project.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Defaults to SCHEMA_VERSION when absent so v1 hand-written files that
    /// predate the field still load; validate.rs enforces it is supported.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub teams: Vec<Team>,
    #[serde(default)]
    pub gates: Vec<Gate>,
    #[serde(default)]
    pub escalations: Vec<Escalation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forks: Vec<Fork>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joins: Vec<Join>,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    /// Path to the team's operating prompt, relative to the project root
    /// (e.g. "prompts/research.md").
    pub prompt: String,
    pub runner: RunnerConfig,
    pub scope: Scope,
    pub outputs: Routes,
    #[serde(default)]
    pub workers: Workers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerConfig {
    pub kind: RunnerKind,
    pub model: String,
    #[serde(default = "default_effort")]
    pub effort: EffortMode,
    /// Only meaningful for the anthropic-api runner kind (v1.1); ignored by
    /// the claude-cli runner. Kept optional so v1 claude-cli teams omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
}

fn default_effort() -> EffortMode {
    EffortMode::Standard
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Scope {
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub writes: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
}

/// A node's routing edges — the bundle of a node's three **Route**s
/// (DOMAIN.md ubiquitous language: a *Route* is one edge — `on_approve` /
/// `on_revise` / `on_reject` — pointing at another node). Named `Routes`
/// (plural) because it groups the three; the YAML key is `outputs` (the
/// spec's own wire name). Each value is the id of a target node (team / gate /
/// escalation) or null/absent (no edge — e.g. an entry team has no on_revise).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Routes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_approve: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_revise: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_reject: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workers {
    pub default: u32,
    pub max: u32,
}

impl Default for Workers {
    fn default() -> Self {
        Self { default: 1, max: 1 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    pub id: String,
    pub label: String,
    /// Where approved files route. The gate's upstream is implied by which
    /// team's on_approve points at this gate (spec → Routing model).
    pub downstream: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Escalation {
    pub id: String,
    #[serde(default)]
    pub triggers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fork {
    pub id: String,
    pub lanes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Join {
    pub id: String,
    pub waits_for: Vec<String>,
    pub downstream: String,
}

/// A node kind discriminator used by validation and the frontend viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Team,
    Gate,
    Escalation,
    Fork,
    Join,
}

impl Pipeline {
    /// All node ids in the pipeline, paired with their kind. Used by validation
    /// (route targets must resolve to one of these) and the viewer.
    pub fn node_ids(&self) -> Vec<(String, NodeKind)> {
        let mut ids = Vec::new();
        ids.extend(self.teams.iter().map(|t| (t.id.clone(), NodeKind::Team)));
        ids.extend(self.gates.iter().map(|g| (g.id.clone(), NodeKind::Gate)));
        ids.extend(self.escalations.iter().map(|e| (e.id.clone(), NodeKind::Escalation)));
        ids.extend(self.forks.iter().map(|f| (f.id.clone(), NodeKind::Fork)));
        ids.extend(self.joins.iter().map(|j| (j.id.clone(), NodeKind::Join)));
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_team(id: &str) -> Team {
        Team {
            id: id.into(),
            name: id.into(),
            prompt: format!("prompts/{id}.md"),
            runner: RunnerConfig {
                kind: RunnerKind::ClaudeCli,
                model: "claude-opus-4-7".into(),
                effort: EffortMode::ExtendedHigh,
                api_key_env: None,
            },
            scope: Scope::default(),
            outputs: Routes::default(),
            workers: Workers::default(),
        }
    }

    #[test]
    fn schema_version_constant_is_two() {
        assert_eq!(SCHEMA_VERSION, 2);
    }

    #[test]
    fn workers_default_is_one_one() {
        assert_eq!(Workers::default(), Workers { default: 1, max: 1 });
    }

    #[test]
    fn node_ids_collects_all_three_kinds() {
        let p = Pipeline {
            id: "p".into(),
            name: "P".into(),
            description: String::new(),
            schema_version: 1,
            teams: vec![sample_team("research")],
            gates: vec![Gate { id: "gate-1".into(), label: "G".into(), downstream: "research".into() }],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![],
            joins: vec![],
        };
        let ids = p.node_ids();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&("research".into(), NodeKind::Team)));
        assert!(ids.contains(&("gate-1".into(), NodeKind::Gate)));
        assert!(ids.contains(&("needs-human".into(), NodeKind::Escalation)));
    }

    #[test]
    fn pipeline_round_trips_through_serde_json() {
        let p = Pipeline {
            id: "p".into(),
            name: "P".into(),
            description: "d".into(),
            schema_version: 1,
            teams: vec![sample_team("research")],
            gates: vec![],
            escalations: vec![],
            forks: vec![],
            joins: vec![],
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: Pipeline = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn node_ids_includes_forks_and_joins() {
        let p = Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 2,
            teams: vec![sample_team("research")],
            gates: vec![],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["a".into(), "b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["a".into(), "b".into()], downstream: "research".into() }],
        };
        let ids = p.node_ids();
        assert!(ids.contains(&("fork-1".into(), NodeKind::Fork)));
        assert!(ids.contains(&("join-1".into(), NodeKind::Join)));
    }

    #[test]
    fn forks_and_joins_default_to_empty_when_absent() {
        let json = r#"{"id":"p","name":"P","schema_version":1,"teams":[],"gates":[],"escalations":[]}"#;
        let p: Pipeline = serde_json::from_str(json).unwrap();
        assert!(p.forks.is_empty());
        assert!(p.joins.is_empty());
    }
}
