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
    /// Whether `--permission-mode auto` works on this model. A model without
    /// it silently runs in `default` mode instead.
    #[serde(default)]
    pub supports_auto_mode: bool,
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
    /// A plugin the team declares cannot be found among the installed plugins
    /// or the directory marketplaces.
    PluginNotFound { name: String },
    /// A scope path cannot be made absolute (an unknown or unbound variable,
    /// or a relative path with no target repo).
    PathUnresolvable { pattern: String },
    /// The team holds the Remote git grant but its model has no auto mode, so
    /// no classifier would judge whether the task asked for remote actions.
    RemoteGitWithoutAutoMode,
}

impl ModelList {
    pub fn find(&self, value: &str) -> Option<&ModelOption> {
        self.models.iter().find(|m| m.value == value)
    }

    /// Whether the listed model supports auto mode. An unlisted model does not.
    pub fn supports_auto_mode(&self, model: &str) -> bool {
        self.find(model).is_some_and(|m| m.supports_auto_mode)
    }

    /// The permission mode a worker on `model` runs in: `auto` where the model
    /// supports it, otherwise `acceptEdits`.
    pub fn permission_mode(&self, model: &str) -> crate::PermissionMode {
        if self.supports_auto_mode(model) {
            crate::PermissionMode::Auto
        } else {
            crate::PermissionMode::AcceptEdits
        }
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
                    supports_auto_mode: false,
                },
                ModelOption {
                    value: "haiku".into(),
                    resolved_model: Some("claude-haiku-4-5-20251001".into()),
                    display_name: "Haiku 4.5".into(),
                    description: None,
                    effort_levels: vec![],
                    supports_auto_mode: false,
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
        assert_eq!(keys, ["description", "display_name", "effort_levels", "resolved_model", "supports_auto_mode", "value"]);
        assert!(v.as_object().unwrap().contains_key("cli_version"));
        assert_eq!(serde_json::to_value(ModelListSource::Live).unwrap(), "live");
        assert_eq!(serde_json::to_value(ModelListSource::Cached).unwrap(), "cached");
        let p = serde_json::to_value(RunnerConfigProblem::EffortNotSupported { level: "max".into() }).unwrap();
        assert_eq!(p, serde_json::json!({"kind": "effort_not_supported", "level": "max"}));
    }

    #[test]
    fn the_permission_mode_follows_supports_auto_mode() {
        let mut l = list();
        l.models[0].supports_auto_mode = true;
        assert!(l.supports_auto_mode("opus"));
        assert!(!l.supports_auto_mode("haiku"));
        assert!(!l.supports_auto_mode("claude-nope"));
        assert_eq!(l.permission_mode("opus"), crate::PermissionMode::Auto);
        assert_eq!(l.permission_mode("haiku"), crate::PermissionMode::AcceptEdits);
        assert_eq!(l.permission_mode("claude-nope"), crate::PermissionMode::AcceptEdits);
    }

    #[test]
    fn a_cached_option_without_the_flag_reads_as_unsupported() {
        let m: ModelOption = serde_json::from_value(serde_json::json!({
            "value": "opus", "resolved_model": null, "display_name": "Opus", "description": null, "effort_levels": []
        }))
        .unwrap();
        assert!(!m.supports_auto_mode);
    }

    #[test]
    fn scope_problems_serialise_in_snake_case() {
        assert_eq!(
            serde_json::to_value(RunnerConfigProblem::PluginNotFound { name: "x".into() }).unwrap(),
            serde_json::json!({"kind": "plugin_not_found", "name": "x"})
        );
        assert_eq!(
            serde_json::to_value(RunnerConfigProblem::PathUnresolvable { pattern: "p".into() }).unwrap(),
            serde_json::json!({"kind": "path_unresolvable", "pattern": "p"})
        );
        assert_eq!(
            serde_json::to_value(RunnerConfigProblem::RemoteGitWithoutAutoMode).unwrap(),
            serde_json::json!({"kind": "remote_git_without_auto_mode"})
        );
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
