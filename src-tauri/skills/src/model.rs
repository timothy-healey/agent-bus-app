//! The `SkillEntry` DTO + the small enums it carries + the `SkillCatalog` trait.
//! `SkillEntry` is the ONLY type that crosses the trait boundary (and the IPC
//! boundary to the frontend); the serde shape is pinned by a wire-contract test.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What kind of `/`-invokable thing this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillKind {
    Skill,
    Command,
}

/// Which `.claude` root an entry came from. Drives the popover source tag and
/// the collision precedence (project wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillSource {
    Global,
    Project,
}

/// A single discoverable skill or slash command. The only type that crosses the
/// `SkillCatalog` seam and the IPC boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillEntry {
    /// Bare invocation name, e.g. "ddd-council", "init-session".
    pub name: String,
    pub kind: SkillKind,
    /// Plugin name, for the "plugin:skill" qualified form. None for direct
    /// (project-local / user) skills + commands.
    pub namespace: Option<String>,
    /// From SKILL.md / command frontmatter (may be empty).
    pub description: String,
    /// Best-effort declared verbs; `[]` when none recognized.
    pub verbs: Vec<String>,
    pub source: SkillSource,
    /// True when this entry must be inserted under its qualified `namespace:name`
    /// form because the bare name is ambiguous (cross-plugin) or lost a
    /// cross-source collision to a project entry. Set by `merge_with_precedence`.
    #[serde(default)]
    pub qualified: bool,
}

impl SkillEntry {
    /// The `/`-token an author should insert for this entry (no leading slash):
    /// `namespace:name` when `qualified` and a namespace exists, else bare `name`.
    pub fn insert_form(&self) -> String {
        match (&self.namespace, self.qualified) {
            (Some(ns), true) => format!("{ns}:{}", self.name),
            _ => self.name.clone(),
        }
    }
}

/// A `.claude` directory to scan, tagged by which source it represents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeRoot {
    pub path: PathBuf,
    pub source: SkillSource,
}

/// The discovery seam. Real impl = `FsSkillScanner` (walks the filesystem);
/// fake = `FakeSkillCatalog` (canned entries). Filesystem paths and SKILL.md
/// parsing never cross this trait — only `SkillEntry`.
pub trait SkillCatalog: Send + Sync {
    /// Scan each root and return its entries. A missing root contributes
    /// nothing (never errors). Source-tagging + cross-source precedence is the
    /// caller's job (`merge_with_precedence`).
    fn list(&self, roots: &[ClaudeRoot]) -> Vec<SkillEntry>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, ns: Option<&str>) -> SkillEntry {
        SkillEntry {
            name: name.into(),
            kind: SkillKind::Skill,
            namespace: ns.map(|s| s.into()),
            description: String::new(),
            verbs: vec![],
            source: SkillSource::Global,
            qualified: false,
        }
    }

    #[test]
    fn insert_form_is_bare_when_not_qualified() {
        let e = entry("brainstorming", Some("superpowers"));
        assert_eq!(e.insert_form(), "brainstorming");
    }

    #[test]
    fn insert_form_is_qualified_when_flagged_and_namespaced() {
        let mut e = entry("brainstorming", Some("superpowers"));
        e.qualified = true;
        assert_eq!(e.insert_form(), "superpowers:brainstorming");
    }

    #[test]
    fn insert_form_stays_bare_when_qualified_but_no_namespace() {
        let mut e = entry("init-session", None);
        e.qualified = true;
        assert_eq!(e.insert_form(), "init-session");
    }
}
