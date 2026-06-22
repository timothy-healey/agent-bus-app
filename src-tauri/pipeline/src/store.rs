//! Filesystem persistence for pipeline definitions. YAML files live under
//! <project_root>/pipelines/ (Workspace path-resolution kernel owns the dir
//! name). The store reads, lists, writes (with validation), and instantiates
//! bundled templates.

use crate::model::Pipeline;
use crate::parse::{parse_pipeline, PipelineParseError};
use crate::template::Template;
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
        validate(&pipeline)?;
        Ok(pipeline)
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
        validate(pipeline)?;
        let dir = pipelines_dir(&self.project_root);
        std::fs::create_dir_all(&dir)?;
        let yaml = serde_yaml::to_string(pipeline)?;
        std::fs::write(self.yaml_path(&pipeline.id), yaml)?;
        Ok(())
    }

    /// Copy a bundled template's YAML into the project verbatim, then load it
    /// back (which validates). Used by the project wizard / activation flow.
    pub fn instantiate_template(&self, template: &Template) -> Result<Pipeline, PipelineStoreError> {
        let dir = pipelines_dir(&self.project_root);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(self.yaml_path(template.id), template.yaml)?;
        self.load(template.id)
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::bundled_templates;

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
    fn instantiate_then_load_and_list_round_trip() {
        let store = PipelineStore::new(temp_root());
        let template = bundled_templates().into_iter().next().unwrap();

        let p = store.instantiate_template(&template).unwrap();
        assert_eq!(p.id, "ddd-spec-plan-impl");

        let ids = store.list_ids().unwrap();
        assert_eq!(ids, vec!["ddd-spec-plan-impl".to_string()]);

        let reloaded = store.load("ddd-spec-plan-impl").unwrap();
        assert_eq!(reloaded.teams.len(), 7);
    }

    #[test]
    fn save_round_trips_through_yaml() {
        let store = PipelineStore::new(temp_root());
        let template = bundled_templates().into_iter().next().unwrap();
        let mut p = crate::parse::parse_pipeline(template.yaml).unwrap();
        p.name = "Renamed".into();

        store.save(&p).unwrap();
        let reloaded = store.load(&p.id).unwrap();
        assert_eq!(reloaded.name, "Renamed");
    }
}
