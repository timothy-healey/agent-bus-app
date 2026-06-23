//! DraftPipeline — an in-progress, NOT-yet-valid pipeline the wizard edits
//! (DOMAIN.md → Pipeline Authoring). Distinct from the validated `Pipeline`
//! aggregate (vet F2): a half-built draft cannot satisfy validate.rs, so the
//! wizard manipulates this looser structure. Best-effort validation surfaces
//! issues live; only a draft that passes HARD validation (validate::validate on
//! its to_pipeline()) becomes a `Pipeline`. Prompt text is held inline as
//! `prompt_body`; to_pipeline() converts it to a `prompts/<id>.md` path.

use crate::model::{Escalation, Fork, Gate, Join, Pipeline, Routes, RunnerConfig, Scope, Team, Workers, SCHEMA_VERSION};
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
    pub gates: Vec<Gate>,
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
            gates: vec![],
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
    #[serde(default)]
    pub gates: Vec<Gate>,
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
            draft.gates = s.gates;
        }
    }
}

/// Non-blocking validation for LIVE editing (Decision D3). Returns a list of
/// human-readable issues; NEVER errors and never blocks. The wizard shows these
/// inline. Hard validation (validate::validate on to_pipeline()) is what gates
/// the create. An empty draft is not an error — it reports the single "no teams
/// yet" hint.
pub fn best_effort_validate(draft: &DraftPipeline) -> Vec<String> {
    let mut issues = Vec::new();
    if draft.teams.is_empty() {
        issues.push("draft has no teams yet".to_string());
        return issues;
    }

    // Known node ids: teams + forks + joins + escalations.
    let mut known: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for t in &draft.teams { known.insert(t.id.as_str()); }
    for f in &draft.forks { known.insert(f.id.as_str()); }
    for j in &draft.joins { known.insert(j.id.as_str()); }
    for e in &draft.escalations { known.insert(e.id.as_str()); }
    for g in &draft.gates { known.insert(g.id.as_str()); }

    for t in &draft.teams {
        if t.prompt_body.trim().is_empty() {
            issues.push(format!("team '{}' has no prompt yet", t.id));
        }
        for (label, target) in [
            ("on_approve", t.outputs.on_approve.as_deref()),
            ("on_revise", t.outputs.on_revise.as_deref()),
            ("on_reject", t.outputs.on_reject.as_deref()),
        ] {
            if let Some(target) = target {
                if !known.contains(target) {
                    issues.push(format!("team '{}' {} points at unknown node '{}'", t.id, label, target));
                }
            }
        }
    }
    for f in &draft.forks {
        for lane in &f.lanes {
            if !known.contains(lane.as_str()) {
                issues.push(format!("fork '{}' lane '{}' is not a known team", f.id, lane));
            }
        }
    }
    for j in &draft.joins {
        for w in &j.waits_for {
            if !known.contains(w.as_str()) {
                issues.push(format!("join '{}' waits_for '{}' is not a known team", j.id, w));
            }
        }
        if !known.contains(j.downstream.as_str()) {
            issues.push(format!("join '{}' downstream '{}' is unknown", j.id, j.downstream));
        }
    }
    for g in &draft.gates {
        if !known.contains(g.downstream.as_str()) {
            issues.push(format!("gate '{}' downstream '{}' is unknown", g.id, g.downstream));
        }
    }
    issues
}

/// The relative prompt path for a team (`prompts/<id>.md`). Single source of the
/// path convention so to_pipeline() and prompt_files() agree.
fn prompt_path(team_id: &str) -> String {
    format!("prompts/{team_id}.md")
}

impl DraftPipeline {
    /// Convert to a real `Pipeline` (Decision D1/D5). Each team's inline
    /// prompt_body becomes a prompts/<id>.md path; gates carry through (W3).
    /// The result is NOT yet validated — the caller runs
    /// hard validation (validate::validate) before writing anything.
    pub fn to_pipeline(&self) -> Pipeline {
        Pipeline {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            schema_version: self.schema_version,
            defaults: None,
            teams: self
                .teams
                .iter()
                .map(|t| Team {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    prompt: prompt_path(&t.id),
                    // R5: the wizard authors a full RunnerConfig per team; wrap it
                    // as a complete TeamRunnerConfig override.
                    runner: Some(crate::model::TeamRunnerConfig::from_full(t.runner.clone())),
                    scope: t.scope.clone(),
                    outputs: t.outputs.clone(),
                    workers: t.workers.clone(),
                })
                .collect(),
            gates: self.gates.clone(),
            escalations: self.escalations.clone(),
            forks: self.forks.clone(),
            joins: self.joins.clone(),
        }
    }
}

/// The per-team prompt files to write: `("prompts/<id>.md", body)` for every
/// team. Workspace writes these (vet F1); Pipeline Authoring only produces them.
pub fn prompt_files(draft: &DraftPipeline) -> Vec<(String, String)> {
    draft
        .teams
        .iter()
        .map(|t| (prompt_path(&t.id), t.prompt_body.clone()))
        .collect()
}

/// Serialize a validated Pipeline to YAML (reuses serde_yaml, the same shape the
/// PipelineStore writes). Pipeline Authoring serializes; Workspace writes (F1).
pub fn to_yaml(pipeline: &Pipeline) -> Result<String, serde_yaml::Error> {
    serde_yaml::to_string(pipeline)
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
    fn empty_draft_has_no_gates() {
        assert!(DraftPipeline::empty().gates.is_empty());
    }

    #[test]
    fn draft_with_gates_round_trips_through_serde_json() {
        use crate::model::Gate;
        let mut d = DraftPipeline::empty();
        d.gates.push(Gate { id: "gate-2".into(), label: "Plan review".into(), downstream: "implementers".into() });
        let s = serde_json::to_string(&d).unwrap();
        let back: DraftPipeline = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
        assert_eq!(back.gates[0].id, "gate-2");
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
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["a".into(), "b".into()], downstream: "needs-human".into(), cancel_on_reject: false, quorum: None }],
            gates: vec![],
        }));
        assert_eq!(d.teams.iter().find(|t| t.id == "entry").unwrap().outputs.on_approve.as_deref(), Some("fork-1"));
        assert_eq!(d.forks.len(), 1);
        assert_eq!(d.joins[0].downstream, "needs-human");
    }

    #[test]
    fn wiring_slice_replaces_gates_alongside_forks_and_joins() {
        use crate::model::Gate;
        let mut d = DraftPipeline::empty();
        d.teams.push(DraftTeam::new("plan-writers", "Plan Writers"));
        d.teams.push(DraftTeam::new("implementers", "Implementers"));
        apply_slice(&mut d, Slice::Wiring(WiringSlice {
            routes: vec![RouteEdge { team_id: "plan-writers".into(), on_approve: Some("gate-2".into()), on_revise: None, on_reject: None }],
            forks: vec![],
            joins: vec![],
            gates: vec![Gate { id: "gate-2".into(), label: "Plan review".into(), downstream: "implementers".into() }],
        }));
        assert_eq!(d.gates.len(), 1);
        assert_eq!(d.gates[0].downstream, "implementers");
        assert_eq!(d.teams.iter().find(|t| t.id == "plan-writers").unwrap().outputs.on_approve.as_deref(), Some("gate-2"));
    }

    #[test]
    fn best_effort_includes_gates_in_known_nodes() {
        use crate::model::Gate;
        // a team routing to a gate must NOT be flagged as unknown (gates are known nodes)
        let mut d = DraftPipeline::empty();
        let mut a = DraftTeam::new("plan-writers", "Plan Writers");
        a.prompt_body = "x".into();
        a.outputs.on_approve = Some("gate-2".into());
        let mut b = DraftTeam::new("implementers", "Implementers");
        b.prompt_body = "y".into();
        d.teams.push(a);
        d.teams.push(b);
        d.gates.push(Gate { id: "gate-2".into(), label: "G".into(), downstream: "implementers".into() });
        assert_eq!(best_effort_validate(&d), Vec::<String>::new());
    }

    #[test]
    fn best_effort_flags_a_gate_downstream_to_an_unknown_node() {
        use crate::model::Gate;
        let mut d = DraftPipeline::empty();
        let mut a = DraftTeam::new("plan-writers", "Plan Writers");
        a.prompt_body = "x".into();
        d.teams.push(a);
        d.gates.push(Gate { id: "gate-2".into(), label: "G".into(), downstream: "ghost".into() });
        let issues = best_effort_validate(&d);
        assert!(issues.iter().any(|i| i.contains("gate-2") && i.contains("ghost")));
    }

    #[test]
    fn best_effort_allows_a_gate_inside_a_fork_lane() {
        use crate::model::{Fork, Gate, Join};
        // P1: gates (and nested forks) may sit inside a lane; best-effort no longer
        // flags it. (Hard validate confirms hierarchical reachability + depth.)
        let mut d = DraftPipeline::empty();
        let mut entry = DraftTeam::new("entry", "Entry");
        entry.prompt_body = "x".into();
        entry.outputs.on_approve = Some("fork-1".into());
        let mut la = DraftTeam::new("lane-a", "Lane A");
        la.prompt_body = "x".into();
        la.outputs.on_approve = Some("gate-x".into()); // gate inside the lane
        let mut lb = DraftTeam::new("lane-b", "Lane B");
        lb.prompt_body = "x".into();
        lb.outputs.on_approve = Some("join-1".into());
        let mut after = DraftTeam::new("after", "After");
        after.prompt_body = "x".into();
        d.teams.push(entry);
        d.teams.push(la);
        d.teams.push(lb);
        d.teams.push(after);
        d.forks.push(Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] });
        d.joins.push(Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into(), cancel_on_reject: false, quorum: None });
        d.gates.push(Gate { id: "gate-x".into(), label: "X".into(), downstream: "join-1".into() });
        let issues = best_effort_validate(&d);
        assert!(
            !issues.iter().any(|i| i.contains("no gates inside a lane")),
            "gate-in-lane is allowed under P1; issues were: {issues:?}"
        );
    }

    #[test]
    fn best_effort_on_empty_draft_reports_no_teams_only() {
        let issues = best_effort_validate(&DraftPipeline::empty());
        assert_eq!(issues, vec!["draft has no teams yet".to_string()]);
    }

    #[test]
    fn best_effort_flags_a_team_with_no_prompt() {
        let mut d = DraftPipeline::empty();
        d.teams.push(DraftTeam::new("research", "Research"));
        let issues = best_effort_validate(&d);
        assert!(issues.iter().any(|i| i.contains("research") && i.contains("prompt")));
    }

    #[test]
    fn best_effort_flags_a_route_to_an_unknown_node() {
        let mut d = DraftPipeline::empty();
        let mut t = DraftTeam::new("research", "Research");
        t.prompt_body = "x".into();
        t.outputs.on_approve = Some("ghost".into());
        d.teams.push(t);
        let issues = best_effort_validate(&d);
        assert!(issues.iter().any(|i| i.contains("ghost")));
    }

    #[test]
    fn best_effort_is_silent_on_a_complete_linear_draft() {
        let mut d = DraftPipeline::empty();
        let mut a = DraftTeam::new("research", "Research");
        a.prompt_body = "investigate".into();
        a.outputs.on_approve = Some("writers".into());
        let mut b = DraftTeam::new("writers", "Writers");
        b.prompt_body = "write".into();
        d.teams.push(a);
        d.teams.push(b);
        assert_eq!(best_effort_validate(&d), Vec::<String>::new());
    }

    fn complete_draft() -> DraftPipeline {
        let mut d = DraftPipeline::empty();
        d.id = "demo".into();
        d.name = "Demo".into();
        d.description = "A two-team demo".into();
        let mut a = DraftTeam::new("research", "Research");
        a.prompt_body = "You investigate the repo.".into();
        a.outputs.on_approve = Some("writers".into());
        let mut b = DraftTeam::new("writers", "Writers");
        b.prompt_body = "You write the spec.".into();
        d.teams.push(a);
        d.teams.push(b);
        d
    }

    #[test]
    fn to_pipeline_maps_prompt_body_to_a_prompts_path() {
        let p = complete_draft().to_pipeline();
        assert_eq!(p.id, "demo");
        assert_eq!(p.teams[0].prompt, "prompts/research.md");
        assert_eq!(p.teams[1].prompt, "prompts/writers.md");
        // gates is always empty for a draft-built pipeline (D1)
        assert!(p.gates.is_empty());
    }

    #[test]
    fn to_pipeline_emits_the_drafts_gates_and_hard_validates() {
        use crate::model::Gate;
        let mut d = DraftPipeline::empty();
        d.id = "demo".into();
        d.name = "Demo".into();
        let mut a = DraftTeam::new("plan-writers", "Plan Writers");
        a.prompt_body = "write the plan".into();
        a.outputs.on_approve = Some("gate-2".into());
        let mut b = DraftTeam::new("implementers", "Implementers");
        b.prompt_body = "implement".into();
        d.teams.push(a);
        d.teams.push(b);
        d.gates.push(Gate { id: "gate-2".into(), label: "Plan review".into(), downstream: "implementers".into() });
        let p = d.to_pipeline();
        assert_eq!(p.gates.len(), 1);
        assert_eq!(p.gates[0].downstream, "implementers");
        // the gate makes implementers reachable -> hard validate passes
        assert_eq!(crate::validate::validate(&p), Ok(()));
    }

    #[test]
    fn to_pipeline_then_hard_validate_passes_for_a_complete_draft() {
        let p = complete_draft().to_pipeline();
        assert_eq!(crate::validate::validate(&p), Ok(()));
    }

    #[test]
    fn to_pipeline_wraps_team_runner_as_some_full_override() {
        let p = complete_draft().to_pipeline();
        let tr = p.teams[0].runner.as_ref().unwrap();
        assert!(tr.kind.is_some() && tr.model.is_some() && tr.effort.is_some());
        // and it resolves + hard-validates
        let resolved = crate::resolve::resolve_defaults(&p);
        assert_eq!(crate::validate::validate(&resolved), Ok(()));
    }

    #[test]
    fn prompt_files_are_team_id_addressed_markdown() {
        let files = prompt_files(&complete_draft());
        assert_eq!(files.len(), 2);
        assert!(files.contains(&("prompts/research.md".to_string(), "You investigate the repo.".to_string())));
        assert!(files.contains(&("prompts/writers.md".to_string(), "You write the spec.".to_string())));
    }

    #[test]
    fn to_yaml_round_trips_through_parse() {
        let p = complete_draft().to_pipeline();
        let yaml = to_yaml(&p).unwrap();
        let back = crate::parse::parse_pipeline(&yaml).unwrap();
        assert_eq!(back.id, "demo");
        assert_eq!(back.teams.len(), 2);
        assert_eq!(back.teams[0].prompt, "prompts/research.md");
    }
}
