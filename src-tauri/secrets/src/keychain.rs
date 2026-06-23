//! KeychainStore — the secure secret-store seam for v1.1. A tiny dedicated
//! infrastructure concern: store/get/delete a secret by (service, account).
//! Behind a trait so no domain context depends on the OS Security API and tests
//! use FakeKeychain. Scope for v1.1: API-key storage only.
//!
//! `account` is the OS-keychain term (kSecAttrAccount); it is the OS-facing
//! seam's word and stays here. The app-facing surface (secrets::api) speaks
//! `key_id` and maps it to this `account` slot (vet F2).

use std::collections::HashMap;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum KeychainError {
    #[error("keychain item not found")]
    NotFound,
    #[error("keychain error: {0}")]
    Backend(String),
}

/// A secure secret store. Real impl = OS keychain; fake = in-memory.
pub trait KeychainStore: Send + Sync {
    /// Store (or overwrite) the secret for (service, account).
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), KeychainError>;
    /// Read the secret. NotFound when absent.
    fn get(&self, service: &str, account: &str) -> Result<String, KeychainError>;
    /// Delete the secret. Idempotent — deleting an absent item is Ok.
    fn delete(&self, service: &str, account: &str) -> Result<(), KeychainError>;
}

/// In-memory KeychainStore for tests. Keyed by "service\0account".
#[derive(Default)]
pub struct FakeKeychain {
    items: Mutex<HashMap<String, String>>,
}

impl FakeKeychain {
    pub fn new() -> Self {
        Self::default()
    }
    fn key(service: &str, account: &str) -> String {
        format!("{service}\0{account}")
    }
}

impl KeychainStore for FakeKeychain {
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), KeychainError> {
        self.items
            .lock()
            .unwrap()
            .insert(Self::key(service, account), secret.to_string());
        Ok(())
    }
    fn get(&self, service: &str, account: &str) -> Result<String, KeychainError> {
        self.items
            .lock()
            .unwrap()
            .get(&Self::key(service, account))
            .cloned()
            .ok_or(KeychainError::NotFound)
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), KeychainError> {
        self.items.lock().unwrap().remove(&Self::key(service, account));
        Ok(())
    }
}

/// Real macOS keychain impl using the Security framework. Stores secrets as
/// generic passwords keyed by (service, account) in the user's login keychain.
/// NOTE: this hits the live OS keychain — it cannot run headless, so its
/// round-trip test is #[ignore]d. The seam + FakeKeychain carry full coverage.
#[cfg(target_os = "macos")]
pub struct SecurityFrameworkKeychain;

#[cfg(target_os = "macos")]
impl SecurityFrameworkKeychain {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(target_os = "macos")]
impl Default for SecurityFrameworkKeychain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "macos")]
impl KeychainStore for SecurityFrameworkKeychain {
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), KeychainError> {
        use security_framework::passwords::{delete_generic_password, set_generic_password};
        // Overwrite semantics: delete any existing item first (ignore absence).
        let _ = delete_generic_password(service, account);
        set_generic_password(service, account, secret.as_bytes())
            .map_err(|e| KeychainError::Backend(e.to_string()))
    }
    fn get(&self, service: &str, account: &str) -> Result<String, KeychainError> {
        use security_framework::passwords::get_generic_password;
        match get_generic_password(service, account) {
            Ok(bytes) => String::from_utf8(bytes).map_err(|e| KeychainError::Backend(e.to_string())),
            Err(_) => Err(KeychainError::NotFound),
        }
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), KeychainError> {
        use security_framework::passwords::delete_generic_password;
        // Idempotent: a missing item is not an error.
        let _ = delete_generic_password(service, account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_set_get_round_trip() {
        let kc = FakeKeychain::new();
        kc.set("agent-bus-app", "anthropic-api", "sk-123").unwrap();
        assert_eq!(kc.get("agent-bus-app", "anthropic-api").unwrap(), "sk-123");
    }

    #[test]
    fn fake_get_missing_is_not_found() {
        let kc = FakeKeychain::new();
        assert!(matches!(kc.get("s", "a"), Err(KeychainError::NotFound)));
    }

    #[test]
    fn fake_set_overwrites() {
        let kc = FakeKeychain::new();
        kc.set("s", "a", "one").unwrap();
        kc.set("s", "a", "two").unwrap();
        assert_eq!(kc.get("s", "a").unwrap(), "two");
    }

    #[test]
    fn fake_delete_is_idempotent() {
        let kc = FakeKeychain::new();
        kc.delete("s", "a").unwrap(); // absent
        kc.set("s", "a", "x").unwrap();
        kc.delete("s", "a").unwrap();
        assert!(matches!(kc.get("s", "a"), Err(KeychainError::NotFound)));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_live_tests {
    use super::*;

    #[test]
    #[ignore = "hits the live login keychain; run manually with --ignored"]
    fn real_keychain_round_trip() {
        let kc = SecurityFrameworkKeychain::new();
        kc.set("agent-bus-app-test", "s1-roundtrip", "sk-live").unwrap();
        assert_eq!(
            kc.get("agent-bus-app-test", "s1-roundtrip").unwrap(),
            "sk-live"
        );
        kc.delete("agent-bus-app-test", "s1-roundtrip").unwrap();
        assert!(matches!(
            kc.get("agent-bus-app-test", "s1-roundtrip"),
            Err(KeychainError::NotFound)
        ));
    }
}
