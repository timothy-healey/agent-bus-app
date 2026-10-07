//! runners — the Runners anti-corruption layer (ACL). Translates a Runtime
//! (task + scope + prompt) into Claude's idiom (CLI flags / stream-json) and
//! Claude's responses back into Runtime's idiom (Structured output, usage).
//! The `Runner` trait is the seam: real subprocess spawning lives only in
//! ClaudeCliRunner; everything else is pure and fixture-tested.


pub mod anthropic_api;
pub mod api;
pub mod claude_cli;
pub mod command;
pub mod control_request;
pub mod curated_models;
pub mod fake;
pub mod model_query;
pub mod output;
pub mod scope;
pub mod stream_json;
pub mod usage_query;

pub use output::*;
