//! conversational_control — the god terminal. A CUSTOMER of all six supplier
//! contexts: it consumes the union of their published `tools()` ToolSpecs to
//! build an in-memory, read-only tool catalog, and dispatches tool calls to the
//! owning supplier's command via the ToolDispatcher seam (concrete impl lives at
//! the composition root, the only place that imports every context). It owns one
//! aggregate — the Conversation — persisted in the `conversations` table.
//!
//! Dependency rule (D1): this crate depends on NO supplier crate. The
//! customer→supplier call crosses the ToolDispatcher trait, never a direct
//! import.

pub mod turn;
pub mod conversation;
pub mod catalog;
pub mod dispatch;
pub mod command;
pub mod engine;
pub mod summarise;
pub mod store;
pub mod api;

#[cfg(test)]
mod contract_tests;
