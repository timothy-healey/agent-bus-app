//! workspace — the Workspace context. Owns Project state + path resolution
//! (path resolution lands in a later task; this file initially exposes only
//! the Project type and store).

pub mod project;

pub use project::*;
