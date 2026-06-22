//! agent_bus_core — shared kernel for cross-context primitives.
//!
//! Per DOMAIN.md and the design spec, this crate holds the ID newtypes,
//! cross-context enums, and OHS protocol types. It depends on nothing
//! project-internal.

pub mod ids;

pub use ids::*;
