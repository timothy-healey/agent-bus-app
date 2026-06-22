use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerKind {
    ClaudeCli,
    AnthropicApi,
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
}
