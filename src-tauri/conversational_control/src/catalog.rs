//! ToolCatalog — the union of every supplier's published ToolSpecs. Built once
//! at the composition root (D2) from `runtime::api::tools()` ++ `review::api::
//! tools()` ++ … and handed to TerminalState. In-memory, read-only after
//! construction, never persisted (D3, spec "Tool catalog ownership").

use agent_bus_core::ToolSpec;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct ToolCatalog {
    specs: Vec<ToolSpec>,
}

impl ToolCatalog {
    /// Build from the concatenated union of every supplier's `tools()`.
    pub fn new(specs: Vec<ToolSpec>) -> Self {
        ToolCatalog { specs }
    }

    pub fn len(&self) -> usize { self.specs.len() }
    pub fn is_empty(&self) -> bool { self.specs.is_empty() }
    pub fn specs(&self) -> &[ToolSpec] { &self.specs }

    /// Look up a tool by its name. Names are globally unique across suppliers
    /// (the dispatcher relies on this; see `duplicate_names`).
    pub fn by_name(&self, name: &str) -> Option<&ToolSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Group tool names by supplier_context slug (for the ⌘K command palette).
    pub fn grouped_by_context(&self) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for s in &self.specs {
            out.entry(s.supplier_context.clone()).or_default().push(s.name.clone());
        }
        out
    }

    /// Any tool names that appear more than once (a catalog-construction bug —
    /// two suppliers publishing the same name). Empty in a healthy catalog.
    pub fn duplicate_names(&self) -> Vec<String> {
        let mut seen = BTreeMap::<&str, usize>::new();
        for s in &self.specs {
            *seen.entry(s.name.as_str()).or_default() += 1;
        }
        seen.into_iter().filter(|(_, n)| *n > 1).map(|(k, _)| k.to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(name: &str, ctx: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: "d".into(),
            input_schema: json!({"type": "object"}),
            supplier_context: ctx.into(),
        }
    }

    #[test]
    fn by_name_finds_a_published_tool() {
        let cat = ToolCatalog::new(vec![spec("inject_topic", "runtime"), spec("add_comment", "review")]);
        assert_eq!(cat.by_name("inject_topic").unwrap().supplier_context, "runtime");
        assert!(cat.by_name("nope").is_none());
    }

    #[test]
    fn grouped_by_context_buckets_names() {
        let cat = ToolCatalog::new(vec![
            spec("inject_topic", "runtime"),
            spec("approve_gate", "runtime"),
            spec("add_comment", "review"),
        ]);
        let g = cat.grouped_by_context();
        assert_eq!(g["runtime"], vec!["inject_topic", "approve_gate"]);
        assert_eq!(g["review"], vec!["add_comment"]);
    }

    #[test]
    fn duplicate_names_detects_collisions() {
        let cat = ToolCatalog::new(vec![spec("x", "runtime"), spec("x", "review")]);
        assert_eq!(cat.duplicate_names(), vec!["x"]);
        let healthy = ToolCatalog::new(vec![spec("a", "runtime"), spec("b", "review")]);
        assert!(healthy.duplicate_names().is_empty());
    }
}
