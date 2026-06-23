//! Secrets OHS — Tauri commands for managing the per-runner API key in the
//! keychain.
//!
//! INVARIANT (no-secret-egress): the secret is WRITE-ONLY across the boundary —
//! `runner_set_api_key` / `runner_clear_api_key` cross it, and
//! `runner_get_api_key_status` returns presence ONLY. No command ever returns
//! the stored secret to the frontend; that is the whole reason the key lives in
//! the keychain rather than a SQLite column. A future `get_api_key` command
//! would violate this invariant.
//!
//! The OHS/IPC parameter is `key_id` (the runner-key identifier, default
//! "anthropic-api"); the OS-keychain term `account` is sealed inside the
//! KeychainStore impls and `key_id` is mapped to the OS `account` slot here.

use crate::keychain::KeychainStore;
use agent_bus_core::ToolSpec;
use serde_json::json;
use std::sync::Arc;

/// The keychain service namespace for all this app's secrets.
pub const SERVICE: &str = "agent-bus-app";

pub struct KeychainState {
    pub store: Arc<dyn KeychainStore>,
}

/// Inner logic (testable without a Tauri State wrapper). `key_id` is the
/// app-facing runner-key identifier; it maps to the OS `account` slot.
pub fn set_api_key_inner(store: &dyn KeychainStore, key_id: &str, key: &str) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("api key must not be empty".into());
    }
    store.set(SERVICE, key_id, key).map_err(|e| e.to_string())
}

pub fn api_key_status_inner(store: &dyn KeychainStore, key_id: &str) -> bool {
    store.get(SERVICE, key_id).is_ok()
}

pub fn clear_api_key_inner(store: &dyn KeychainStore, key_id: &str) -> Result<(), String> {
    store.delete(SERVICE, key_id).map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn runner_set_api_key(
    state: tauri::State<'_, KeychainState>,
    key_id: String,
    key: String,
) -> Result<(), String> {
    set_api_key_inner(state.store.as_ref(), &key_id, &key)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn runner_get_api_key_status(
    state: tauri::State<'_, KeychainState>,
    key_id: String,
) -> Result<bool, String> {
    Ok(api_key_status_inner(state.store.as_ref(), &key_id))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn runner_clear_api_key(
    state: tauri::State<'_, KeychainState>,
    key_id: String,
) -> Result<(), String> {
    clear_api_key_inner(state.store.as_ref(), &key_id)
}

/// OHS contract. The secret value never appears in a tool schema — these tools
/// describe the management surface, not the secret.
pub fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "runner_get_api_key_status".into(),
        description: "Report whether a runner API key is stored in the keychain (presence only)."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": { "key_id": { "type": "string" } },
            "required": ["key_id"]
        }),
        supplier_context: "secrets".into(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keychain::FakeKeychain;

    #[test]
    fn set_then_status_is_true_and_clear_removes() {
        let kc = FakeKeychain::new();
        assert!(!api_key_status_inner(&kc, "anthropic-api"));
        set_api_key_inner(&kc, "anthropic-api", "sk-1").unwrap();
        assert!(api_key_status_inner(&kc, "anthropic-api"));
        clear_api_key_inner(&kc, "anthropic-api").unwrap();
        assert!(!api_key_status_inner(&kc, "anthropic-api"));
    }

    #[test]
    fn empty_key_is_rejected() {
        let kc = FakeKeychain::new();
        assert!(set_api_key_inner(&kc, "anthropic-api", "   ").is_err());
    }

    #[test]
    fn status_only_returns_presence_never_the_secret() {
        let kc = FakeKeychain::new();
        set_api_key_inner(&kc, "anthropic-api", "sk-secret").unwrap();
        // The status surface is a bool; there is no path that returns the secret.
        let present: bool = api_key_status_inner(&kc, "anthropic-api");
        assert!(present);
    }
}
