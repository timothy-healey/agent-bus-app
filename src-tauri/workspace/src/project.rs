use agent_bus_core::{PipelineId, ProjectId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub root_path: PathBuf,
    pub active_pipeline_id: Option<PipelineId>,
    pub created_at: i64,    // unix epoch seconds
    pub updated_at: i64,
}

impl Project {
    /// Create a new in-memory Project. Caller is responsible for persistence
    /// (see ProjectStore in api.rs).
    pub fn new(name: String, root_path: PathBuf, now_unix: i64) -> Self {
        Self {
            id: ProjectId(format!("proj-{}", uuid::Uuid::new_v4())),
            name,
            root_path,
            active_pipeline_id: None,
            created_at: now_unix,
            updated_at: now_unix,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_new_generates_proj_prefixed_id() {
        let p = Project::new("Test".into(), "/tmp/test".into(), 1_700_000_000);
        assert!(p.id.0.starts_with("proj-"), "id was {}", p.id.0);
        assert_eq!(p.name, "Test");
        assert_eq!(p.root_path, PathBuf::from("/tmp/test"));
        assert_eq!(p.active_pipeline_id, None);
        assert_eq!(p.created_at, 1_700_000_000);
        assert_eq!(p.updated_at, 1_700_000_000);
    }

    #[test]
    fn project_round_trips_through_serde() {
        let p = Project::new("X".into(), "/p".into(), 1);
        let s = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
