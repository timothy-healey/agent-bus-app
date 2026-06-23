//! llm_chat — the multi-turn Claude dialogue anti-corruption layer (ACL).
//!
//! Unlike `runners` (one-shot, verdict-shaped worker invocations), this ACL
//! provides a *chat*: many turns over a stable dialogue, returning the
//! assistant's *reply text* + usage. Two consumers use it without coupling to
//! each other — the god terminal (Conversational Control) and the brainstorming
//! wizard (Pipeline Authoring, sub-project 3).
//!
//! Dependency rule (F2): this crate depends on NOTHING project-internal except
//! `agent_bus_core` (the kernel). It must never import `runners`, `pipeline`,
//! `runtime`, `usage_telemetry`, `workspace`, or `conversational_control`.
//!
//! Boundary discipline (F3): Claude-CLI idioms (`session_id`, `--resume`) are
//! sealed inside this crate. Callers pass a stable `dialogue_id`; `ChatReply`
//! carries only domain-shaped data (text + usage). Session continuity lives in
//! `session.rs` and never crosses out.

pub mod chat;
pub mod command;
pub mod stream_json;
pub mod session;
pub mod claude_cli;
pub mod fake;

pub use chat::*;
