//! Runners OHS. The ACL exposes no operator app-tools in v1 (it has no
//! commands the god terminal would call — runner selection is a composition
//! concern, not an operator action). tools() returns an empty list so Plan 6's
//! catalog union and the composition root treat every context uniformly.

use agent_bus_core::ToolSpec;

pub fn tools() -> Vec<ToolSpec> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runners_publishes_no_tools_in_v1() {
        assert!(tools().is_empty());
    }
}
