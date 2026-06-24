//! runtime — the Runtime context. Owns the Task lifecycle state machine
//! (Task aggregate), the WorkerPool (per-team tokio workers; second aggregate,
//! joined to Task by reference), the Pipeline router, and the system brake.

pub mod task;

pub use task::*;

pub mod task_store;

pub use task_store::*;

pub mod worker;

pub use worker::*;

pub mod router;
pub mod brake;

pub use router::*;
pub use brake::*;

pub mod pool;

pub use pool::*;

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
