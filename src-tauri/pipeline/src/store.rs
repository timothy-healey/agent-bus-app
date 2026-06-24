//! Filesystem persistence for pipeline definitions. YAML files live under
//! <project_root>/pipelines/ (Workspace path-resolution kernel owns the dir
//! name). The store reads, lists, and writes (with validation).

use crate::model::Pipeline;
use crate::parse::{parse_pipeline, PipelineParseError};
use crate::validate::{validate, PipelineValidationError};
use std::path::{Path, PathBuf};
use thiserror::Error;
use workspace::paths::pipelines_dir;

#[derive(Debug, Error)]
pub enum PipelineStoreError {
    #[error("pipeline not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Parse(#[from] PipelineParseError),
    #[error(transparent)]
    Validation(#[from] PipelineValidationError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialize error: {0}")]
    Serialize(#[from] serde_yaml::Error),
}

pub struct PipelineStore {
    project_root: PathBuf,
}

impl PipelineStore {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self { project_root: project_root.into() }
    }

    fn yaml_path(&self, id: &str) -> PathBuf {
        pipelines_dir(&self.project_root).join(format!("{id}.yaml"))
    }

    /// Load + validate the pipeline with the given id.
    pub fn load(&self, id: &str) -> Result<Pipeline, PipelineStoreError> {
        let path = self.yaml_path(id);
        if !path.exists() {
            return Err(PipelineStoreError::NotFound(id.to_string()));
        }
        let yaml = std::fs::read_to_string(&path)?;
        let pipeline = parse_pipeline(&yaml)?;
        // R5: resolve pipeline defaults into each team BEFORE validating/returning
        // so Runtime only ever sees fully-specified team runners (resolution stays
        // a Pipeline Authoring concern — DOMAIN.md).
        let resolved = crate::resolve::resolve_defaults(&pipeline);
        validate(&resolved)?;
        Ok(resolved)
    }

    /// List the ids of all *.yaml pipelines in the project's pipelines/ dir.
    /// Returns an empty Vec when the directory doesn't exist yet.
    pub fn list_ids(&self) -> Result<Vec<String>, PipelineStoreError> {
        let dir = pipelines_dir(&self.project_root);
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    ids.push(stem.to_string());
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Validate, then write a pipeline to <root>/pipelines/<id>.yaml. Refuses
    /// to write an invalid graph (spec → Pipeline editor: "Invalid graphs …
    /// refuse to save with a clear error").
    pub fn save(&self, pipeline: &Pipeline) -> Result<(), PipelineStoreError> {
        // R5: validate the RESOLVED form (a team may legitimately omit fields it
        // inherits from pipeline defaults) but persist the AUTHORED form so
        // defaults + omitted team runners stay compact on disk.
        let resolved = crate::resolve::resolve_defaults(pipeline);
        validate(&resolved)?;
        let dir = pipelines_dir(&self.project_root);
        std::fs::create_dir_all(&dir)?;
        let yaml = serde_yaml::to_string(pipeline)?;
        std::fs::write(self.yaml_path(&pipeline.id), yaml)?;
        Ok(())
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("abp-pipeline-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn list_ids_empty_when_no_pipelines_dir() {
        let store = PipelineStore::new(temp_root());
        assert_eq!(store.list_ids().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn load_missing_returns_not_found() {
        let store = PipelineStore::new(temp_root());
        assert!(matches!(store.load("nope"), Err(PipelineStoreError::NotFound(_))));
    }

    #[test]
    fn save_round_trips_through_yaml() {
        use crate::model::{Escalation, Pipeline, Role, Routes, Scope, Team, TeamRunnerConfig, Workers};
        use agent_bus_core::{EffortMode, RunnerKind};
        let store = PipelineStore::new(temp_root());
        let p = Pipeline {
            id: "demo".into(), name: "Demo".into(), description: String::new(), schema_version: 1,
            defaults: None,
            teams: vec![Team {
                id: "research".into(), name: "Research".into(), prompt: "prompts/research.md".into(),
                runner: Some(TeamRunnerConfig { kind: Some(RunnerKind::ClaudeCli), model: Some("m".into()), effort: Some(EffortMode::Standard), api_key_env: None }),
                scope: Scope::default(),
                outputs: Routes { on_approve: Some("needs-human".into()), on_revise: None, on_reject: None },
                workers: Workers::default(),
                role: Role::default(),
            }],
            gates: vec![],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![], joins: vec![],
        };
        store.save(&p).unwrap();
        let reloaded = store.load("demo").unwrap();
        assert_eq!(reloaded.name, "Demo");
    }

    #[test]
    fn load_resolves_pipeline_defaults_into_each_team() {
        use crate::model::{Escalation, Pipeline, PipelineDefaults, Role, Routes, Scope, Team, Workers};
        use agent_bus_core::{EffortMode, RunnerKind};
        let store = PipelineStore::new(temp_root());
        // author a pipeline whose team omits its runner; pipeline default supplies it
        let p = Pipeline {
            id: "demo".into(), name: "Demo".into(), description: String::new(),
            schema_version: 2,
            defaults: Some(PipelineDefaults {
                default_runner: Some(RunnerKind::ClaudeCli),
                default_model: Some("claude-opus-4-8".into()),
                default_effort: Some(EffortMode::ExtendedHigh),
            }),
            teams: vec![Team {
                id: "research".into(), name: "Research".into(), prompt: "prompts/research.md".into(),
                runner: None, // inherits the whole default
                scope: Scope::default(),
                outputs: Routes { on_approve: Some("needs-human".into()), on_revise: None, on_reject: None },
                workers: Workers::default(),
                role: Role::default(),
            }],
            gates: vec![],
            escalations: vec![Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![], joins: vec![],
        };
        // write the *authored* YAML directly to disk
        let dir = workspace::paths::pipelines_dir(store.project_root());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("demo.yaml"), serde_yaml::to_string(&p).unwrap()).unwrap();

        let loaded = store.load("demo").unwrap();
        let er = loaded.teams[0].effective_runner();
        assert_eq!(er.model, "claude-opus-4-8");
        assert_eq!(er.effort, EffortMode::ExtendedHigh);
    }
}
