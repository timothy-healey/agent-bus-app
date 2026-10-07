//! The models the installed CLI offers and the effort levels each supports.
//! The one source of truth for which Model and Effort combinations are valid.

use crate::Effort;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOption {
    /// What to pass as `--model`.
    pub value: String,
    pub resolved_model: Option<String>,
    pub display_name: String,
    pub description: Option<String>,
    /// Empty when the model takes no effort level.
    pub effort_levels: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelListSource {
    Live,
    Cached,
    Curated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelList {
    pub models: Vec<ModelOption>,
    pub source: ModelListSource,
    pub cli_version: Option<String>,
}

/// Why a team's model/effort pair cannot run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunnerConfigProblem {
    ModelNotAvailable,
    EffortNotSupported { level: String },
}

impl ModelList {
    pub fn find(&self, value: &str) -> Option<&ModelOption> {
        self.models.iter().find(|m| m.value == value)
    }

    /// A model is valid if the list has it; a level is valid if that model
    /// supports it. Default is valid on any listed model.
    pub fn check(&self, model: &str, effort: &Effort) -> Result<(), RunnerConfigProblem> {
        let m = self.find(model).ok_or(RunnerConfigProblem::ModelNotAvailable)?;
        match effort.level() {
            Some(l) if !m.effort_levels.iter().any(|x| x == l) => {
                Err(RunnerConfigProblem::EffortNotSupported { level: l.to_string() })
            }
            _ => Ok(()),
        }
    }
}

/// Where the model list comes from. The Runners ACL implements it against the
/// CLI; tests use a fixed list.
pub trait ModelSource: Send + Sync {
    fn fetch(&self) -> Result<ModelList, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Effort;

    fn list() -> ModelList {
        ModelList {
            models: vec![
                ModelOption {
                    value: "opus".into(),
                    resolved_model: Some("claude-opus-5-5".into()),
                    display_name: "Opus 5.5".into(),
                    description: None,
                    effort_levels: vec!["low".into(), "high".into(), "xhigh".into()],
                },
                ModelOption {
                    value: "haiku".into(),
                    resolved_model: Some("claude-haiku-4-5-20251001".into()),
                    display_name: "Haiku 4.5".into(),
                    description: None,
                    effort_levels: vec![],
                },
            ],
            source: ModelListSource::Curated,
            cli_version: None,
        }
    }

    #[test]
    fn an_unknown_model_is_not_available() {
        assert_eq!(list().check("claude-nope", &Effort::Default), Err(RunnerConfigProblem::ModelNotAvailable));
    }

    #[test]
    fn a_level_the_model_lacks_is_not_supported() {
        assert_eq!(
            list().check("opus", &Effort::Level("max".into())),
            Err(RunnerConfigProblem::EffortNotSupported { level: "max".into() })
        );
    }

    #[test]
    fn default_is_fine_on_a_model_without_effort_support() {
        assert_eq!(list().check("haiku", &Effort::Default), Ok(()));
        assert_eq!(
            list().check("haiku", &Effort::Level("low".into())),
            Err(RunnerConfigProblem::EffortNotSupported { level: "low".into() })
        );
    }

    #[test]
    fn a_supported_level_is_valid() {
        assert_eq!(list().check("opus", &Effort::Level("xhigh".into())), Ok(()));
    }

    #[test]
    fn model_list_json_shape_matches_ts() {
        let v = serde_json::to_value(list()).unwrap();
        assert_eq!(v["source"], "curated");
        let m = &v["models"][0];
        let mut keys: Vec<_> = m.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["description", "display_name", "effort_levels", "resolved_model", "value"]);
        assert!(v.as_object().unwrap().contains_key("cli_version"));
        assert_eq!(serde_json::to_value(ModelListSource::Live).unwrap(), "live");
        assert_eq!(serde_json::to_value(ModelListSource::Cached).unwrap(), "cached");
        let p = serde_json::to_value(RunnerConfigProblem::EffortNotSupported { level: "max".into() }).unwrap();
        assert_eq!(p, serde_json::json!({"kind": "effort_not_supported", "level": "max"}));
    }

    #[test]
    fn model_source_is_object_safe() {
        struct Fixed;
        impl ModelSource for Fixed {
            fn fetch(&self) -> Result<ModelList, String> {
                Ok(list())
            }
        }
        let s: Box<dyn ModelSource> = Box::new(Fixed);
        assert_eq!(s.fetch().unwrap().models.len(), 2);
    }
}
