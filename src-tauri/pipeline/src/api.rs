//! Tauri commands published by the Pipeline Authoring context — its Open Host
//! Service. v1 is read-only over pipelines (the editor is a viewer); the one
//! mutation is instantiating a bundled template, used by project setup.

use crate::model::Pipeline;
use crate::store::PipelineStore;
use crate::template::bundled_templates;
use agent_bus_core::ToolSpec;
use serde::Serialize;
use serde_json::json;

/// A bundled template, in a frontend-friendly shape (no &'static lifetimes).
#[derive(Debug, Clone, Serialize)]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
}

#[tauri::command(rename_all = "snake_case")]
pub fn pipeline_list_templates() -> Vec<TemplateInfo> {
    bundled_templates()
        .into_iter()
        .map(|t| TemplateInfo { id: t.id.to_string(), name: t.name.to_string() })
        .collect()
}

#[tauri::command(rename_all = "snake_case")]
pub fn pipeline_list(project_root: String) -> Result<Vec<String>, String> {
    PipelineStore::new(project_root)
        .list_ids()
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn pipeline_load(project_root: String, id: String) -> Result<Pipeline, String> {
    PipelineStore::new(project_root)
        .load(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn pipeline_instantiate_template(
    project_root: String,
    template_id: String,
) -> Result<Pipeline, String> {
    let template = bundled_templates()
        .into_iter()
        .find(|t| t.id == template_id)
        .ok_or_else(|| format!("unknown template: {template_id}"))?;
    PipelineStore::new(project_root)
        .instantiate_template(&template)
        .map_err(|e| e.to_string())
}

/// OHS contract — consumed by Conversational Control (Plan 6). v1 publishes
/// read-only authoring tools only.
pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "pipeline_list".into(),
            description: "List the pipeline ids defined in a project.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "project_root": { "type": "string" } },
                "required": ["project_root"]
            }),
            supplier_context: "pipeline-authoring".into(),
        },
        ToolSpec {
            name: "pipeline_load".into(),
            description: "Load and validate a pipeline definition by id.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "project_root": { "type": "string" },
                    "id": { "type": "string" }
                },
                "required": ["project_root", "id"]
            }),
            supplier_context: "pipeline-authoring".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_templates_returns_the_ddd_template() {
        let t = pipeline_list_templates();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "ddd-spec-plan-impl");
    }

    #[test]
    fn tools_publishes_pipeline_authoring_named_tools() {
        let t = tools();
        assert!(t.iter().all(|s| s.supplier_context == "pipeline-authoring"));
        assert!(t.iter().any(|s| s.name == "pipeline_list"));
        assert!(t.iter().any(|s| s.name == "pipeline_load"));
    }
}
