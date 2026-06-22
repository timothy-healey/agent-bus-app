//! runners — the Runners anti-corruption layer (ACL). Translates a Runtime
//! (task + scope + prompt) into Claude's idiom (CLI flags / stream-json) and
//! Claude's responses back into Runtime's idiom (verdict, artifact, usage).
//! The `Runner` trait is the seam: real subprocess spawning lives only in
//! ClaudeCliRunner; everything else is pure and fixture-tested.

pub mod api;
pub mod claude_cli;
pub mod command;
pub mod fake;
pub mod output;
pub mod scope;
pub mod stream_json;

pub use output::*;
