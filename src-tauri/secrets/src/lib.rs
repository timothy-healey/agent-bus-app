//! `secrets` — a generic-subdomain infrastructure crate (peer tier to
//! `agent_bus_core`, NOT an eighth bounded context). One job: a secure
//! secret store behind a trait, so no domain context depends on the OS
//! Security API and tests use an in-memory fake. v1.1 scope: API-key storage
//! only (vet F1).

pub mod api;
pub mod keychain;

pub use keychain::{FakeKeychain, KeychainError, KeychainStore};
#[cfg(target_os = "macos")]
pub use keychain::SecurityFrameworkKeychain;
