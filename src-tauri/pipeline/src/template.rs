//! Bundled pipeline templates. v1 ships one: the DDD spec→plan→impl flow.
//! Templates are compiled into the binary via include_str! and copied into a
//! project's pipelines/ directory on instantiation.

use crate::model::Pipeline;
use crate::parse::{parse_pipeline, PipelineParseError};

/// The raw YAML of the bundled DDD template (compiled in).
pub const DDD_TEMPLATE_YAML: &str = include_str!("../templates/ddd-spec-plan-impl.yaml");

/// A bundled template the project wizard can instantiate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// The pipeline id (and the basename used when writing the YAML file).
    pub id: &'static str,
    pub name: &'static str,
    pub yaml: &'static str,
}

/// The catalog of bundled templates.
pub fn bundled_templates() -> Vec<Template> {
    vec![Template {
        id: "ddd-spec-plan-impl",
        name: "DDD Spec → Plan → Implement",
        yaml: DDD_TEMPLATE_YAML,
    }]
}

/// Parse the bundled DDD template into a Pipeline.
pub fn ddd_template() -> Result<Pipeline, PipelineParseError> {
    parse_pipeline(DDD_TEMPLATE_YAML)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::validate;

    #[test]
    fn bundled_ddd_template_parses() {
        let p = ddd_template().unwrap();
        assert_eq!(p.id, "ddd-spec-plan-impl");
        assert_eq!(p.teams.len(), 7);
        assert_eq!(p.gates.len(), 2);
        assert_eq!(p.escalations.len(), 1);
    }

    #[test]
    fn bundled_ddd_template_is_valid() {
        // The bundled template MUST pass the same validation a user file does.
        let p = ddd_template().unwrap();
        assert_eq!(validate(&p), Ok(()));
    }

    #[test]
    fn catalog_contains_the_ddd_template() {
        let t = bundled_templates();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "ddd-spec-plan-impl");
    }
}
