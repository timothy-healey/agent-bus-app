//! runners — the Runners anti-corruption layer (ACL). Translates a Runtime
//! (task + scope + prompt) into Claude's idiom (CLI flags / stream-json) and
//! Claude's responses back into Runtime's idiom (verdict, artifact, usage).
//! The `Runner` trait is the seam: real subprocess spawning lives only in
//! ClaudeCliRunner; everything else is pure and fixture-tested.
//!
//! S3 (EXPERIMENTAL · macOS-only · STRUCTURAL-ONLY): the Scope policy also
//! projects a team Scope to a macOS `sandbox-exec` (SBPL) profile
//! (`scope::sandbox_profile`); when enabled (default OFF) the ClaudeCliRunner
//! wraps the subprocess argv in `sandbox-exec -p <profile>`. The SBPL idiom is
//! sealed inside this ACL — it never crosses the `Runner` trait. `sandbox-exec`
//! is Apple-DEPRECATED (still functional). ONLY profile-generation + argv-
//! wrapping are tested; the live OS confinement is NOT a proven security
//! boundary in this codebase.

pub mod anthropic_api;
pub mod api;
pub mod claude_cli;
pub mod command;
pub mod fake;
pub mod output;
pub mod scope;
pub mod stream_json;

pub use output::*;
