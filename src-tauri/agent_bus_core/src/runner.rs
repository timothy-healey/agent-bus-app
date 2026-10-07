use serde::{Deserialize, Serialize};

/// The model a new team, design session, or god-terminal turn uses when none is
/// chosen. Saved pipelines keep whatever model they stored.
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerKind {
    ClaudeCli,
    AnthropicApi,
}

/// The model new teams use: the CLI's `default` alias, which tracks the
/// recommended model.
pub const DEFAULT_TEAM_MODEL: &str = "default";

/// How hard the model reasons: a CLI effort level, or Default (no `--effort`
/// flag; the model's own default applies). Levels are open strings so a new CLI
/// level needs no app release; the model list decides which are valid.
///
/// Serialised as a bare string; Default is never written (fields skip it). A
/// map is the retired thinking-budget shape and reads as Default.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Effort {
    #[default]
    Default,
    Level(String),
}

impl Effort {
    pub fn is_default(&self) -> bool {
        matches!(self, Effort::Default)
    }

    pub fn level(&self) -> Option<&str> {
        match self {
            Effort::Default => None,
            Effort::Level(l) => Some(l),
        }
    }
}

/// `skip_serializing_if` for optional effort fields: Default is never written.
pub fn effort_is_unset(e: &Option<Effort>) -> bool {
    e.as_ref().map_or(true, Effort::is_default)
}

impl Serialize for Effort {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Effort::Default => s.serialize_none(),
            Effort::Level(l) => s.serialize_str(l),
        }
    }
}

impl<'de> Deserialize<'de> for Effort {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Effort;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an effort level string")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Effort, E> {
                Ok(Effort::Level(v.to_string()))
            }

            // A map is the retired thinking-budget shape; it carries no level.
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut m: A) -> Result<Effort, A::Error> {
                while m.next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?.is_some() {}
                Ok(Effort::Default)
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum EffortMode {
    Off,
    Standard,
    ExtendedLow,
    ExtendedHigh,
    Custom { budget_tokens: u32 },
}

impl EffortMode {
    /// Resolve to the actual thinking-token budget Claude should use.
    pub fn budget_tokens(self) -> u32 {
        match self {
            EffortMode::Off => 0,
            EffortMode::Standard => 1024,
            EffortMode::ExtendedLow => 8192,
            EffortMode::ExtendedHigh => 32000,
            EffortMode::Custom { budget_tokens } => budget_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_kind_serialises_as_kebab_case() {
        assert_eq!(serde_json::to_string(&RunnerKind::ClaudeCli).unwrap(), "\"claude-cli\"");
        assert_eq!(serde_json::to_string(&RunnerKind::AnthropicApi).unwrap(), "\"anthropic-api\"");
    }

    #[test]
    fn effort_mode_preset_budgets() {
        assert_eq!(EffortMode::Off.budget_tokens(), 0);
        assert_eq!(EffortMode::Standard.budget_tokens(), 1024);
        assert_eq!(EffortMode::ExtendedLow.budget_tokens(), 8192);
        assert_eq!(EffortMode::ExtendedHigh.budget_tokens(), 32000);
        assert_eq!(EffortMode::Custom { budget_tokens: 16000 }.budget_tokens(), 16000);
    }

    #[test]
    fn effort_mode_serialises_with_internal_tag() {
        let e = EffortMode::Standard;
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"mode\":\"standard\""), "got: {s}");
    }

    #[test]
    fn default_model_is_opus_5_5() {
        assert_eq!(DEFAULT_MODEL, "claude-opus-5-5");
    }

    #[test]
    fn effort_level_serialises_as_a_bare_string() {
        assert_eq!(serde_json::to_value(Effort::Level("high".into())).unwrap(), serde_json::json!("high"));
    }

    #[test]
    fn effort_deserialises_a_string_as_a_level() {
        let e: Effort = serde_json::from_str("\"xhigh\"").unwrap();
        assert_eq!(e, Effort::Level("xhigh".into()));
    }

    #[test]
    fn every_legacy_mode_map_deserialises_as_default() {
        for legacy in [
            r#"{"mode":"off"}"#,
            r#"{"mode":"standard"}"#,
            r#"{"mode":"extended-low"}"#,
            r#"{"mode":"extended-high"}"#,
            r#"{"mode":"custom","budget_tokens":16000}"#,
            r#"{}"#,
        ] {
            let e: Effort = serde_json::from_str(legacy).unwrap();
            assert_eq!(e, Effort::Default, "{legacy}");
        }
    }

    #[test]
    fn a_non_string_non_map_effort_is_an_error() {
        assert!(serde_json::from_str::<Effort>("42").is_err());
        assert!(serde_json::from_str::<Effort>("true").is_err());
        assert!(serde_json::from_str::<Effort>("[\"high\"]").is_err());
    }

    #[test]
    fn a_struct_field_omits_default_and_reads_a_missing_key_as_default() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Holder {
            #[serde(default, skip_serializing_if = "Effort::is_default")]
            effort: Effort,
        }
        assert_eq!(serde_json::to_string(&Holder { effort: Effort::Default }).unwrap(), "{}");
        let h: Holder = serde_json::from_str("{}").unwrap();
        assert_eq!(h.effort, Effort::Default);
        let h: Holder = serde_json::from_str(r#"{"effort":"low"}"#).unwrap();
        assert_eq!(h.effort, Effort::Level("low".into()));
    }

    #[test]
    fn effort_is_unset_covers_none_and_default() {
        assert!(effort_is_unset(&None));
        assert!(effort_is_unset(&Some(Effort::Default)));
        assert!(!effort_is_unset(&Some(Effort::Level("high".into()))));
    }

    #[test]
    fn default_team_model_is_the_default_alias() {
        assert_eq!(DEFAULT_TEAM_MODEL, "default");
    }
}
