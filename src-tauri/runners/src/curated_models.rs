//! The built-in model list: the fallback when the CLI cannot be queried and
//! nothing is cached. Mirrors a real `initialize` capture
//! (`fixtures/initialize-sample.jsonl`); the live list always wins.

use agent_bus_core::{ModelList, ModelListSource, ModelOption};

const ALL: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const NO_XHIGH: &[&str] = &["low", "medium", "high", "max"];

fn m(value: &str, resolved: &str, display: &str, description: &str, levels: &[&str]) -> ModelOption {
    ModelOption {
        value: value.into(),
        resolved_model: Some(resolved.into()),
        display_name: display.into(),
        description: Some(description.into()),
        effort_levels: levels.iter().map(|l| l.to_string()).collect(),
    }
}

pub fn curated() -> ModelList {
    ModelList {
        models: vec![
            m("default", "claude-opus-5-5", "Default (recommended)", "Opus 5.5 · Best for everyday, complex tasks", ALL),
            m("opus", "claude-opus-5-5", "Opus 5.5", "For complex work and everyday tasks", ALL),
            m("fable", "claude-fable-5-1", "Fable 5.1", "For your toughest challenges", ALL),
            m("sonnet", "claude-sonnet-5-5", "Sonnet 5.5", "Most efficient for simpler tasks", ALL),
            m("haiku", "claude-haiku-4-5-20251001", "Haiku 4.5", "Fastest for quick answers", &[]),
            m("claude-sonnet-5", "claude-sonnet-5", "Sonnet 5", "Efficient for routine tasks", ALL),
            m("claude-opus-5", "claude-opus-5", "Opus 5", "Best for everyday, complex tasks", ALL),
            m("claude-fable-5", "claude-fable-5", "Fable 5", "Most capable for your hardest and longest-running tasks", ALL),
            m("claude-opus-4-8", "claude-opus-4-8", "Opus 4.8", "Best for everyday, complex tasks", ALL),
            m("claude-opus-4-7", "claude-opus-4-7", "Opus 4.7", "Best for everyday, complex tasks", ALL),
            m("claude-opus-4-6", "claude-opus-4-6", "Opus 4.6", "Best for everyday, complex tasks", NO_XHIGH),
            m("claude-sonnet-4-6", "claude-sonnet-4-6", "Sonnet 4.6", "Efficient for routine tasks", NO_XHIGH),
        ],
        source: ModelListSource::Curated,
        cli_version: None,
    }
}
