//! workspace — the Workspace context.

pub mod project;
pub mod store;
pub mod api;
pub mod git_config;
pub mod paths;

pub use project::*;
pub use store::*;

#[cfg(test)]
mod contract_tests;
