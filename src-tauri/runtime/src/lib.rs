//! runtime — the Runtime context. Owns the Task lifecycle state machine
//! (Task aggregate), the bounded-buffer execution `engine` (per-run worker
//! loops, joined to Task by reference), and the system brake.

pub mod task;

pub use task::*;

pub mod task_store;

pub use task_store::*;

pub mod worker;

pub use worker::*;

pub mod brake;

pub use brake::*;

pub mod log_sink;

pub use log_sink::*;

pub mod fanout_group;
pub use fanout_group::*;

pub mod fanout_store;
pub use fanout_store::*;

pub mod invocation_audit;
pub use invocation_audit::*;

pub mod store;
pub use store::*;

pub mod run_store;
pub use run_store::*;

pub mod generator_ledger;
pub use generator_ledger::*;

pub mod engine;
pub use engine::*;

pub mod revision;

pub mod api;

#[cfg(test)]
mod contract_tests;
