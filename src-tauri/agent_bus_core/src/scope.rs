//! Scope vocabulary shared by Pipeline Authoring, Runners and Runtime: what a
//! team may be granted beyond the always-on tools, which permission mode its
//! worker runs in, and what a worker was refused.

use serde::{Deserialize, Serialize};

/// A capability a team may be granted on top of the always-on tools (Read,
/// Glob, Grep, Skill, Edit, Write). Written in YAML as a short string:
/// `bash`, `bash(<pattern>)`, `agent`, `web-fetch`, `web-search`, `remote-git`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ToolGrant {
    /// Every shell command.
    Bash,
    /// Shell commands matching one CLI permission pattern, e.g. `git diff:*`.
    BashPattern(String),
    /// Spawning subagents.
    Agent,
    WebFetch,
    WebSearch,
    /// Pushing, fetching, pulling and `gh`, judged per task by auto mode.
    RemoteGit,
}

impl ToolGrant {
    /// Parse the YAML form. Case-insensitive on the keyword; a pattern keeps
    /// its own case. `None` for anything else.
    pub fn parse(s: &str) -> Option<ToolGrant> {
        let t = s.trim();
        let lower = t.to_ascii_lowercase();
        match lower.as_str() {
            "bash" => return Some(ToolGrant::Bash),
            "agent" => return Some(ToolGrant::Agent),
            "web-fetch" | "webfetch" => return Some(ToolGrant::WebFetch),
            "web-search" | "websearch" => return Some(ToolGrant::WebSearch),
            "remote-git" => return Some(ToolGrant::RemoteGit),
            _ => {}
        }
        if lower.starts_with("bash(") && t.ends_with(')') {
            let inner = t[5..t.len() - 1].trim();
            if !inner.is_empty() {
                return Some(ToolGrant::BashPattern(inner.to_string()));
            }
        }
        None
    }

    /// The YAML form.
    pub fn as_string(&self) -> String {
        match self {
            ToolGrant::Bash => "bash".into(),
            ToolGrant::BashPattern(p) => format!("bash({p})"),
            ToolGrant::Agent => "agent".into(),
            ToolGrant::WebFetch => "web-fetch".into(),
            ToolGrant::WebSearch => "web-search".into(),
            ToolGrant::RemoteGit => "remote-git".into(),
        }
    }

    /// Read one entry of the legacy `tools:` list: `Bash`, `Bash(...)`,
    /// `Agent`, `WebFetch` and `WebSearch` map to grants; anything else (the
    /// always-on tools, unknown names) maps to nothing.
    pub fn from_legacy_tool(s: &str) -> Option<ToolGrant> {
        let t = s.trim();
        let lower = t.to_ascii_lowercase();
        match lower.as_str() {
            "bash" => Some(ToolGrant::Bash),
            "agent" | "task" => Some(ToolGrant::Agent),
            "webfetch" => Some(ToolGrant::WebFetch),
            "websearch" => Some(ToolGrant::WebSearch),
            _ if lower.starts_with("bash(") => ToolGrant::parse(t),
            _ => None,
        }
    }
}

impl Serialize for ToolGrant {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.as_string())
    }
}

impl<'de> Deserialize<'de> for ToolGrant {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        ToolGrant::parse(&s).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown grant '{s}' (expected bash, bash(<pattern>), agent, web-fetch, web-search or remote-git)"
            ))
        })
    }
}

/// The permission mode a worker runs in: `auto` where the model supports it,
/// otherwise `acceptEdits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Auto,
    AcceptEdits,
}

impl PermissionMode {
    /// The `--permission-mode` value, which is also what the CLI reports back.
    pub fn as_cli(self) -> &'static str {
        match self {
            PermissionMode::Auto => "auto",
            PermissionMode::AcceptEdits => "acceptEdits",
        }
    }
}

/// What refused an action: a permission rule, or the auto-mode classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DenialSource {
    Rule,
    Classifier,
}

/// An action a worker attempted and was refused. Reported, never fatal. A
/// tool the team was not granted is absent rather than denied, so it never
/// shows here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDenial {
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub source: DenialSource,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_grant_round_trips_through_its_string_form() {
        for g in [
            ToolGrant::Bash,
            ToolGrant::BashPattern("git diff:*".into()),
            ToolGrant::Agent,
            ToolGrant::WebFetch,
            ToolGrant::WebSearch,
            ToolGrant::RemoteGit,
        ] {
            let v = serde_json::to_value(&g).unwrap();
            let back: ToolGrant = serde_json::from_value(v).unwrap();
            assert_eq!(back, g);
        }
        assert_eq!(serde_json::to_value(ToolGrant::BashPattern("git diff:*".into())).unwrap(), json!("bash(git diff:*)"));
        assert_eq!(serde_json::to_value(ToolGrant::RemoteGit).unwrap(), json!("remote-git"));
        assert_eq!(serde_json::to_value(ToolGrant::WebFetch).unwrap(), json!("web-fetch"));
    }

    #[test]
    fn the_keyword_is_case_insensitive_and_the_pattern_keeps_its_case() {
        assert_eq!(ToolGrant::parse("Bash(git log:*)"), Some(ToolGrant::BashPattern("git log:*".into())));
        assert_eq!(ToolGrant::parse("BASH"), Some(ToolGrant::Bash));
        assert_eq!(ToolGrant::parse("bash(Make X)"), Some(ToolGrant::BashPattern("Make X".into())));
        assert_eq!(ToolGrant::parse("bash()"), None);
    }

    #[test]
    fn an_unknown_grant_is_a_deserialize_error() {
        assert!(serde_json::from_value::<ToolGrant>(json!("teleport")).is_err());
        assert!(serde_json::from_value::<ToolGrant>(json!("Read")).is_err());
    }

    #[test]
    fn legacy_tools_map_to_grants_and_the_rest_is_dropped() {
        assert_eq!(ToolGrant::from_legacy_tool("Bash"), Some(ToolGrant::Bash));
        assert_eq!(ToolGrant::from_legacy_tool("Bash(git commit:*)"), Some(ToolGrant::BashPattern("git commit:*".into())));
        assert_eq!(ToolGrant::from_legacy_tool("Agent"), Some(ToolGrant::Agent));
        assert_eq!(ToolGrant::from_legacy_tool("WebFetch"), Some(ToolGrant::WebFetch));
        assert_eq!(ToolGrant::from_legacy_tool("WebSearch"), Some(ToolGrant::WebSearch));
        for dropped in ["Read", "Write", "Edit", "Glob", "Grep", "NotebookEdit", "whatever"] {
            assert_eq!(ToolGrant::from_legacy_tool(dropped), None, "{dropped}");
        }
    }

    #[test]
    fn permission_mode_cli_values() {
        assert_eq!(PermissionMode::Auto.as_cli(), "auto");
        assert_eq!(PermissionMode::AcceptEdits.as_cli(), "acceptEdits");
    }

    #[test]
    fn a_denial_serialises_with_a_lowercase_source() {
        let d = PermissionDenial { tool_name: "Bash".into(), tool_input: json!({"command": "git push"}), source: DenialSource::Rule };
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            json!({"tool_name": "Bash", "tool_input": {"command": "git push"}, "source": "rule"})
        );
        let c: PermissionDenial = serde_json::from_value(json!({"tool_name":"Bash","tool_input":{},"source":"classifier"})).unwrap();
        assert_eq!(c.source, DenialSource::Classifier);
    }
}
