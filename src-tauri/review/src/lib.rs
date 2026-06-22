//! Review bounded context: artifacts, comments, and recorded verdicts.
//!
//! Conformist to Runtime — Review records the operator's comments and a
//! verdict marker, but never mutates Task state (Runtime's `*_gate` commands
//! own the state machine).

pub mod api;
pub mod comment;
pub mod store;
