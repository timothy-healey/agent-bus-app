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
    /// Pipeline-level runner defaults teams inherit (R5). None = no defaults;
    /// every team must then specify its own runner. Resolved at load
    /// (resolve.rs) so Runtime only ever sees fully-specified team runners.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<PipelineDefaults>,
    #[serde(default)]
    pub teams: Vec<Team>,
    #[serde(default)]
    pub gates: Vec<Gate>,
    #[serde(default)]
    pub escalations: Vec<Escalation>,
    #[serde(default)]
    pub forks: Vec<Fork>,
    #[serde(default)]
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
    /// As authored: a partial override over `Pipeline.defaults`, or None to
    /// inherit the whole default (R5). After `resolve::resolve_defaults` every
    /// team holds `Some(fully-specified)`; use `effective_runner()` to read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<TeamRunnerConfig>,
    pub scope: Scope,
    pub outputs: Routes,
    #[serde(default)]
    pub workers: Workers,
    /// Producer vs reviewer (vet F8). Default producer; see `Role`.
    #[serde(default)]
    pub role: Role,
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

/// Pipeline-level runner defaults (R5). Every field optional: a pipeline may
/// supply just a model, just a runner kind, etc. Teams that omit a field
/// inherit it (Pipeline Authoring resolves; see resolve.rs). Additive — a
/// pipeline with no `defaults` key loads unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PipelineDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_runner: Option<RunnerKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<EffortMode>,
}

/// A team's runner config as authored (R5): every field optional so a team can
/// override just the model and inherit kind+effort from `Pipeline.defaults`.
/// The resolver (resolve.rs) overlays this on the pipeline defaults to produce
/// a fully-specified `RunnerConfig`. A team that omits `runner` entirely
/// inherits the whole default.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TeamRunnerConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<RunnerKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<EffortMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
}

impl TeamRunnerConfig {
    /// Wrap a fully-specified RunnerConfig as a (complete) override — used by
    /// the wizard's `to_pipeline()` where the team runner is always full.
    pub fn from_full(r: RunnerConfig) -> Self {
        Self {
            kind: Some(r.kind),
            model: Some(r.model),
            effort: Some(r.effort),
            api_key_env: r.api_key_env,
        }
    }
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
    /// Minimum live workers for this team's pool (the always-on floor). Renamed
    /// from `default` to align code with the context-map invariant
    /// `workers.count ≥ team.workers.min` (vet F2). UI label: "Scale (min·max)".
    pub min: u32,
    pub max: u32,
}

impl Default for Workers {
    fn default() -> Self {
        Self { min: 1, max: 1 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Fork {
    pub id: String,
    pub lanes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Join {
    pub id: String,
    pub waits_for: Vec<String>,
    pub downstream: String,
    /// Early-cancel policy (P2). When true, the join resolves to needs-human the
    /// moment ONE lane fails (reject / revise-cap), cancelling the outstanding
    /// lanes instead of waiting for the full barrier. `#[serde(default)]` false
    /// keeps existing pipelines on the full-barrier behavior; additive-optional,
    /// no SCHEMA_VERSION bump. The policy is enforced by the FanOutGroup barrier.
    #[serde(default)]
    pub cancel_on_reject: bool,
    /// Quorum policy (P3). When `Some(n)`, the join proceeds to `downstream` as
    /// soon as `n` of its lanes approve (early-resolve on success); if reaching
    /// `n` becomes impossible (unsettled lanes + approvals-so-far < n) it resolves
    /// to needs-human. `None` (default) = all-must-approve (the original barrier).
    /// Additive-optional, no SCHEMA_VERSION bump. The rule lives on the
    /// `FanOutGroup` aggregate. PRECEDENCE: when `quorum` is set it governs
    /// success and `cancel_on_reject` is ignored — a reject only matters insofar
    /// as it can make quorum impossible (see DOMAIN.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quorum: Option<u32>,
}

/// A team's role in the pipeline (graph-builder vet F8). A **reviewer** emits a
/// verdict (approve/revise/reject) on each item it sees; a **producer** (incl.
/// implementers) hands its output forward without a verdict. Additive enum,
/// default `producer` (matches L1 "producers default approve, reviewers judge").
/// Routing/verdict semantics consume this in the runtime-behavior chunk; the
/// graph builder reads it for role-aware edges (replacing the `teamRole` regex).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Producer,
    Reviewer,
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

impl Team {
    /// The team's fully-resolved runner (R5). Only valid after the pipeline has
    /// been resolved (resolve::resolve_defaults) — every field is then present.
    /// Panics if called on an unresolved team (a programmer error: the store
    /// resolves + validates before any consumer sees the pipeline).
    pub fn effective_runner(&self) -> RunnerConfig {
        let tr = self
            .runner
            .as_ref()
            .expect("team runner not resolved (call resolve::resolve_defaults first)");
        RunnerConfig {
            kind: tr.kind.expect("resolved runner missing kind"),
            model: tr.model.clone().expect("resolved runner missing model"),
            effort: tr.effort.expect("resolved runner missing effort"),
            api_key_env: tr.api_key_env.clone(),
        }
    }
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
            runner: Some(TeamRunnerConfig {
                kind: Some(RunnerKind::ClaudeCli),
                model: Some("claude-opus-4-7".into()),
                effort: Some(EffortMode::ExtendedHigh),
                api_key_env: None,
            }),
            scope: Scope::default(),
            outputs: Routes::default(),
            workers: Workers::default(),
            role: Role::default(),
        }
    }

    #[test]
    fn wiring_node_types_derive_json_schema() {
        // Fork/Join/Gate must produce a schema (WiringSlice embeds them).
        let fork = schemars::schema_for!(Fork);
        let s = serde_json::to_string(&fork).unwrap();
        assert!(s.contains("lanes"), "Fork schema should expose lanes: {s}");
        let join = serde_json::to_string(&schemars::schema_for!(Join)).unwrap();
        assert!(join.contains("waits_for") && join.contains("downstream"));
        let gate = serde_json::to_string(&schemars::schema_for!(Gate)).unwrap();
        assert!(gate.contains("downstream") && gate.contains("label"));
    }

    #[test]
    fn schema_version_constant_is_two() {
        assert_eq!(SCHEMA_VERSION, 2);
    }

    #[test]
    fn team_role_defaults_to_producer_when_absent() {
        let json = r#"{"id":"t","name":"T","prompt":"p.md","scope":{},"outputs":{}}"#;
        let t: Team = serde_json::from_str(json).unwrap();
        assert_eq!(t.role, Role::Producer);
    }

    #[test]
    fn role_serializes_lowercase_and_round_trips() {
        assert_eq!(serde_json::to_string(&Role::Reviewer).unwrap(), "\"reviewer\"");
        let r: Role = serde_json::from_str("\"producer\"").unwrap();
        assert_eq!(r, Role::Producer);
    }

    #[test]
    fn workers_default_is_one_one() {
        assert_eq!(Workers::default(), Workers { min: 1, max: 1 });
    }

    #[test]
    fn node_ids_collects_all_three_kinds() {
        let p = Pipeline {
            id: "p".into(),
            name: "P".into(),
            description: String::new(),
            schema_version: 1,
            defaults: None,
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
            defaults: None,
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
            defaults: None,
            teams: vec![sample_team("research")],
            gates: vec![],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["a".into(), "b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["a".into(), "b".into()], downstream: "research".into(), cancel_on_reject: false, quorum: None }],
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

    #[test]
    fn team_runner_config_round_trips_and_is_all_optional() {
        // every field absent => deserialises to all-None
        let empty: TeamRunnerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(empty, TeamRunnerConfig::default());

        let full = TeamRunnerConfig {
            kind: Some(RunnerKind::ClaudeCli),
            model: Some("m".into()),
            effort: Some(EffortMode::Standard),
            api_key_env: None,
        };
        let s = serde_json::to_string(&full).unwrap();
        let back: TeamRunnerConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(full, back);
    }

    #[test]
    fn pipeline_defaults_is_all_optional() {
        let d: PipelineDefaults = serde_yaml::from_str("{}").unwrap();
        assert_eq!(d, PipelineDefaults::default());
        let d2 = PipelineDefaults {
            default_runner: Some(RunnerKind::ClaudeCli),
            default_model: Some("claude-opus-4-8".into()),
            default_effort: Some(EffortMode::ExtendedHigh),
        };
        let s = serde_json::to_string(&d2).unwrap();
        let back: PipelineDefaults = serde_json::from_str(&s).unwrap();
        assert_eq!(d2, back);
    }

    #[test]
    fn effective_runner_returns_the_resolved_config() {
        let mut t = sample_team("research");
        let r = t.effective_runner();
        assert_eq!(r.kind, RunnerKind::ClaudeCli);
        assert_eq!(r.model, "claude-opus-4-7");
        // None => effective_runner panics (unresolved pipeline = programmer error)
        t.runner = None;
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| t.effective_runner())).is_err());
    }

    #[test]
    fn join_cancel_on_reject_defaults_to_false_when_absent() {
        // a join authored without the field loads with the full-barrier default
        let json = r#"{"id":"join-1","waits_for":["a","b"],"downstream":"after"}"#;
        let j: Join = serde_json::from_str(json).unwrap();
        assert!(!j.cancel_on_reject);
    }

    #[test]
    fn join_round_trips_cancel_on_reject_true() {
        let j = Join {
            id: "join-1".into(),
            waits_for: vec!["a".into(), "b".into()],
            downstream: "after".into(),
            cancel_on_reject: true,
            quorum: None,
        };
        let s = serde_json::to_string(&j).unwrap();
        let back: Join = serde_json::from_str(&s).unwrap();
        assert_eq!(j, back);
        assert!(back.cancel_on_reject);
    }

    #[test]
    fn join_quorum_defaults_to_none_when_absent() {
        let json = r#"{"id":"join-1","waits_for":["a","b"],"downstream":"after"}"#;
        let j: Join = serde_json::from_str(json).unwrap();
        assert!(j.quorum.is_none());
        assert!(!j.cancel_on_reject);
    }

    #[test]
    fn join_round_trips_quorum_some() {
        let j = Join {
            id: "join-1".into(),
            waits_for: vec!["a".into(), "b".into(), "c".into()],
            downstream: "after".into(),
            cancel_on_reject: false,
            quorum: Some(2),
        };
        let s = serde_json::to_string(&j).unwrap();
        let back: Join = serde_json::from_str(&s).unwrap();
        assert_eq!(j, back);
        assert_eq!(back.quorum, Some(2));
    }

    #[test]
    fn join_omits_quorum_from_json_when_none() {
        let j = Join { id: "j".into(), waits_for: vec!["a".into(), "b".into()], downstream: "after".into(), cancel_on_reject: false, quorum: None };
        let s = serde_json::to_string(&j).unwrap();
        assert!(!s.contains("quorum"), "None quorum must not be serialized: {s}");
    }

    #[test]
    fn pipeline_defaults_field_defaults_to_none_when_absent() {
        let json = r#"{"id":"p","name":"P","schema_version":2,"teams":[]}"#;
        let p: Pipeline = serde_json::from_str(json).unwrap();
        assert!(p.defaults.is_none());
    }
}
