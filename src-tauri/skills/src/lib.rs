//! `skills` — a generic-subdomain infrastructure crate (peer tier to
//! `agent_bus_core`, NOT an eighth bounded context; same call as `secrets`).
//! One job: discover the skills + slash commands available to a worker by
//! scanning `.claude` roots, behind a `SkillCatalog` trait. The filesystem /
//! `SKILL.md`-parsing idiom is SEALED inside the trait — only the `SkillEntry`
//! DTO crosses. Tests use `FakeSkillCatalog`; the real impl is `FsSkillScanner`.
//!
//! Authoring-time only (A4): this never changes how the worker invokes skills.

pub mod model;
pub mod scanner;
pub mod verbs;

pub use model::{ClaudeRoot, SkillCatalog, SkillEntry, SkillKind, SkillSource};
pub use scanner::FsSkillScanner;

#[cfg(test)]
mod contract_tests;

use agent_bus_core::ToolSpec;
use serde_json::json;
use std::collections::HashMap;

/// OHS contract for the skills generic subdomain. `list_skills` is wired at the
/// composition root (it needs the Project type to resolve sources), but the tool
/// description is owned here.
pub fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "list_skills".into(),
        description: "List the skills + slash commands available to a project's worker (authoring-time autocomplete).".into(),
        input_schema: json!({
            "type": "object",
            "properties": { "project_id": { "type": "string" } },
            "required": ["project_id"]
        }),
        supplier_context: "skills".into(),
    }]
}

/// In-memory `SkillCatalog` for tests / downstream root tests. Returns the same
/// canned entries regardless of `roots` (the seam, not the filesystem, is what
/// downstream code depends on).
#[derive(Default, Clone)]
pub struct FakeSkillCatalog {
    pub entries: Vec<SkillEntry>,
}

impl FakeSkillCatalog {
    pub fn new(entries: Vec<SkillEntry>) -> Self {
        Self { entries }
    }
}

impl SkillCatalog for FakeSkillCatalog {
    fn list(&self, _roots: &[ClaudeRoot]) -> Vec<SkillEntry> {
        self.entries.clone()
    }
}

/// Merge the per-root scan results into one catalog, tag each entry by its
/// root's source, and apply A4 collision precedence:
/// **project wins** on a name collision; the losing entry is kept but offered
/// under its qualified `namespace:name` form (via `qualified = true`).
///
/// This is the composition-root merge logic, but it lives in the crate (pure,
/// `SkillEntry`-only) so it is unit-testable without the `Project` type. The
/// root composition (resolving a `Project`'s sources to `ClaudeRoot`s) stays at
/// the app level — the crate never learns `Project`.
pub fn merge_with_precedence(per_root: Vec<(SkillSource, Vec<SkillEntry>)>) -> Vec<SkillEntry> {
    // Stamp the source onto each entry first.
    let mut all: Vec<SkillEntry> = Vec::new();
    for (source, entries) in per_root {
        for mut e in entries {
            e.source = source;
            all.push(e);
        }
    }

    // Index by bare name to find cross-source collisions.
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, e) in all.iter().enumerate() {
        by_name.entry(e.name.clone()).or_default().push(i);
    }

    for indices in by_name.values() {
        if indices.len() < 2 {
            continue;
        }
        // Does at least one Project-sourced entry contend for this name?
        let has_project = indices
            .iter()
            .any(|&i| all[i].source == SkillSource::Project);
        if !has_project {
            // Pure cross-plugin / within-source ambiguity (all same source):
            // every contender is offered qualified.
            for &i in indices {
                all[i].qualified = true;
            }
            continue;
        }
        // Project wins the bare name; every non-project loser is offered qualified.
        for &i in indices {
            if all[i].source != SkillSource::Project {
                all[i].qualified = true;
            }
        }
    }

    all
}

#[cfg(test)]
mod tools_tests {
    use super::*;

    #[test]
    fn tools_publishes_list_skills_under_skills_context() {
        let t = tools();
        assert!(t.iter().any(|s| s.name == "list_skills" && s.supplier_context == "skills"));
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use crate::model::SkillKind;

    fn entry(name: &str, ns: Option<&str>) -> SkillEntry {
        SkillEntry {
            name: name.into(),
            kind: SkillKind::Skill,
            namespace: ns.map(|s| s.into()),
            description: String::new(),
            verbs: vec![],
            source: SkillSource::Global, // overwritten by merge
            qualified: false,
        }
    }

    fn find<'a>(entries: &'a [SkillEntry], name: &str, source: SkillSource) -> &'a SkillEntry {
        entries
            .iter()
            .find(|e| e.name == name && e.source == source)
            .expect("entry present")
    }

    #[test]
    fn stamps_each_root_source_onto_its_entries() {
        let merged = merge_with_precedence(vec![
            (SkillSource::Global, vec![entry("a", None)]),
            (SkillSource::Project, vec![entry("b", None)]),
        ]);
        assert_eq!(find(&merged, "a", SkillSource::Global).source, SkillSource::Global);
        assert_eq!(find(&merged, "b", SkillSource::Project).source, SkillSource::Project);
    }

    #[test]
    fn project_wins_a_cross_source_collision_global_offered_qualified() {
        let merged = merge_with_precedence(vec![
            (SkillSource::Global, vec![entry("dup", Some("plug"))]),
            (SkillSource::Project, vec![entry("dup", None)]),
        ]);
        // Both kept.
        assert_eq!(merged.iter().filter(|e| e.name == "dup").count(), 2);
        // Project keeps the bare name.
        assert!(!find(&merged, "dup", SkillSource::Project).qualified);
        // Global loser is offered qualified.
        assert!(find(&merged, "dup", SkillSource::Global).qualified);
    }

    #[test]
    fn cross_plugin_ambiguity_within_global_is_qualified() {
        let merged = merge_with_precedence(vec![(
            SkillSource::Global,
            vec![entry("vet", Some("p1")), entry("vet", Some("p2"))],
        )]);
        assert!(merged.iter().filter(|e| e.name == "vet").all(|e| e.qualified));
    }

    #[test]
    fn unique_names_stay_bare() {
        let merged = merge_with_precedence(vec![(
            SkillSource::Global,
            vec![entry("solo", Some("p"))],
        )]);
        assert!(!merged[0].qualified);
    }
}
