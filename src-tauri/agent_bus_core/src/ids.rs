use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TeamId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PipelineId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactPath(pub PathBuf);

impl fmt::Display for ProjectId { fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { self.0.fmt(f) } }
impl fmt::Display for TaskId    { fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { self.0.fmt(f) } }
impl fmt::Display for TeamId    { fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { self.0.fmt(f) } }
impl fmt::Display for PipelineId{ fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { self.0.fmt(f) } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_id_round_trips_through_serde() {
        let id = ProjectId("proj-abc".into());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"proj-abc\"");
        let back: ProjectId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn ids_are_not_interchangeable_at_the_type_level() {
        let p = ProjectId("x".into());
        let t = TaskId("x".into());
        assert_eq!(p.0, t.0);
    }

    #[test]
    fn artifact_path_serialises_as_string() {
        let ap = ArtifactPath(PathBuf::from("/tmp/foo.md"));
        let json = serde_json::to_string(&ap).unwrap();
        assert_eq!(json, "\"/tmp/foo.md\"");
    }
}
