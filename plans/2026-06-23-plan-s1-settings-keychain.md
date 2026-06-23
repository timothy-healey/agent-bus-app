# S1 — Full Settings sections + keychain — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the remaining Settings sections (Runners API-key, Git author, Projects management) plus a secure keychain secret store behind a trait, so the `anthropic-api` runner reads its key from the OS keychain (keychain-first, env fallback) and never persists secrets to SQLite/disk.

**Architecture:** A new small dedicated `secrets` crate owns a `KeychainStore` trait with a real `SecurityFrameworkKeychain` (macOS Security framework via `security-framework`) and an in-memory `FakeKeychain`. The keychain is wired at the **composition root** (`app/src/lib.rs`); the runner factory takes an injected key-resolver closure so the Runners ACL stays sealed (no context depends on the OS API). Git author name/email persist in a new `git_config` SQLite row (mirrors `usage_config`). Projects management reuses Workspace's existing OHS commands (list/get/set-active) plus a new `workspace_remove_project`. Settings is restructured into 5 sections (General, Usage, Runners, Git, Projects) with new IPC wrappers.

**Tech Stack:** Rust (sqlx, tauri, async-trait, security-framework 3.7), React + TypeScript (Tauri IPC), vitest.

---

## Decisions

- **DD1 — Where the keychain seam lives.** A new dedicated `secrets` crate (not folded into `runners` or `workspace`). It is a **generic-subdomain infrastructure crate** — the same architectural tier as `agent_bus_core` (owned by the architecture, depended on at the root, depending on nothing project-internal), **NOT an eighth bounded context** (the seven-context map in DOMAIN.md is unchanged). One job: store/get/delete a secret by `(service, account)`. Behind a `KeychainStore` trait so contexts never depend on the OS API and tests use the fake. v1.1 scope is fixed to **API-key storage only** — no per-team key *policy* lives here. **Chosen.** Rationale (vet F1): keeps the OS coupling in one tiny crate; the trait is the seam; nothing in the domain contexts learns about the keychain.
- **DD2 — Key resolution path.** `runner_for` gains an injected `key_resolver: &dyn Fn(&RunnerConfig) -> Option<String>`. The composition root builds a resolver that reads the **keychain first** (service `"agent-bus-app"`, account = the project/runner key id, default `"anthropic-api"`), then falls back to the env var named by `api_key_env`. The resolved `String` is passed into `AnthropicApiRunner::new` exactly as today — **no keychain/OS type crosses the Runner trait** (ACL seal preserved). **Chosen.**
- **DD3 — Git author storage.** A `git_config` single-row table (id=1, `author_name TEXT`, `author_email TEXT`), migration `009`, read/written by new usage-style commands in a new `workspace::git_config` module (Workspace owns project-adjacent config). v1.1 has no worktree-commit path yet, so the Git section persists the values and notes they will be consumed when worker-commits land. **Chosen** (persist now, consume later — avoids a dead concept while keeping scope sane).
- **DD4 — Projects remove.** Add `ProjectStore::remove` + `workspace_remove_project` OHS command. Projects section reuses `workspace_list_projects` / `workspace_get_project` / `workspace_set_active_pipeline` (already shipped) — **no bypass of Workspace's OHS**. **Chosen.**
- **DD5 — Keychain real-path coverage.** The real `SecurityFrameworkKeychain` hits the live login keychain, which cannot run headless in CI. Tests cover the trait + `FakeKeychain` fully and the resolver wiring; the real `security-framework` calls are **structural-only** (a `#[cfg(target_os = "macos")]` impl, compile-checked, with a `set/get/delete` round-trip test gated behind `#[ignore]` so it never runs unattended). **Chosen.**
- **DD6 — Runners API-key command surface.** New commands `runner_set_api_key(key_id, key)` / `runner_get_api_key_status(key_id)` / `runner_clear_api_key(key_id)` live in a new `secrets::api` module managed at the root (`KeychainState` holding `Arc<dyn KeychainStore>`). The OHS/IPC parameter is named **`key_id`** (the runner-key identifier, default `"anthropic-api"`) — the OS-keychain term `account` stays sealed inside the `KeychainStore` impls (vet F2). `get` returns only a **presence boolean**, never the secret, so the secret never round-trips to the frontend (the no-secret-egress invariant, vet F4). **Chosen.**

---

## File Structure

- `src-tauri/secrets/` — NEW crate. `Cargo.toml`, `src/lib.rs` (trait + re-exports), `src/keychain.rs` (`KeychainStore` trait, `KeychainError`, `FakeKeychain`, `SecurityFrameworkKeychain`), `src/api.rs` (`KeychainState` + three Tauri commands + `tools()`).
- `src-tauri/Cargo.toml` — add `secrets` to workspace members + `security-framework` to `[workspace.dependencies]`.
- `src-tauri/workspace/src/store.rs` — add `ProjectStore::remove`.
- `src-tauri/workspace/src/api.rs` — add `workspace_remove_project` command + tool spec.
- `src-tauri/workspace/src/git_config.rs` — NEW. `GitConfig` struct, `load_git_config`, `set_git_config_inner`, `git_config_get` / `git_config_set` commands, `tools()`.
- `src-tauri/workspace/src/lib.rs` — register `git_config` module.
- `src-tauri/app/migrations/009_git_config.sql` — NEW migration.
- `src-tauri/app/src/lib.rs` — register migration (both lists + bump user_version test 8→9), build keychain + resolver, change `runner_for` signature, manage `KeychainState`, register new commands in `invoke_handler`.
- `src/ipc/secrets.ts` — NEW. `setRunnerApiKey` / `getRunnerApiKeyStatus` / `clearRunnerApiKey`.
- `src/ipc/workspace.ts` — add `setActivePipeline`, `removeProject`, `getGitConfig`, `setGitConfig` + `GitConfig` type.
- `src/components/SettingsView.tsx` — restructure into General / Usage / Runners / Git / Projects sections.
- `src/components/SettingsView.test.tsx` — extend coverage for the new sections.
- `src/App.tsx` — pass projects + new handlers to `SettingsView`.

---

## Task 1: Scaffold the `secrets` crate with the trait + FakeKeychain

**Files:**
- Create: `src-tauri/secrets/Cargo.toml`
- Create: `src-tauri/secrets/src/lib.rs`
- Create: `src-tauri/secrets/src/keychain.rs`
- Modify: `src-tauri/Cargo.toml` (workspace members + deps)

- [ ] **Step 1: Add the crate to the workspace + the security-framework dep**

In `src-tauri/Cargo.toml`, add `"secrets"` to `members` and add to `[workspace.dependencies]`:

```toml
security-framework = "3"
```

- [ ] **Step 2: Write `src-tauri/secrets/Cargo.toml`**

```toml
[package]
name = "secrets"
version.workspace = true
edition.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tauri.workspace = true
agent_bus_core = { path = "../agent_bus_core" }

[target.'cfg(target_os = "macos")'.dependencies]
security-framework.workspace = true
```

- [ ] **Step 3: Write the failing test in `src-tauri/secrets/src/keychain.rs`**

```rust
//! KeychainStore — the secure secret-store seam for v1.1. A tiny dedicated
//! infrastructure concern: store/get/delete a secret by (service, account).
//! Behind a trait so no domain context depends on the OS Security API and tests
//! use FakeKeychain. Scope for v1.1: API-key storage only.

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
```

- [ ] **Step 4: Write `src-tauri/secrets/src/lib.rs`**

```rust
pub mod api;
pub mod keychain;

pub use keychain::{KeychainError, KeychainStore, FakeKeychain};
#[cfg(target_os = "macos")]
pub use keychain::SecurityFrameworkKeychain;
```

(`api` module is added in Task 3 — create a temporary empty `src-tauri/secrets/src/api.rs` with `pub fn tools() -> Vec<agent_bus_core::ToolSpec> { vec![] }` for now so the crate compiles; Task 3 fills it in.)

- [ ] **Step 5: Run the tests**

Run: `cargo test -p secrets`
Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/secrets src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(secrets): KeychainStore trait + FakeKeychain (S1)"
```

---

## Task 2: Real macOS SecurityFrameworkKeychain (structural)

**Files:**
- Modify: `src-tauri/secrets/src/keychain.rs`

- [ ] **Step 1: Add the macOS impl (compile-checked; live test ignored)**

Append to `src-tauri/secrets/src/keychain.rs`:

```rust
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
            Ok(bytes) => String::from_utf8(bytes)
                .map_err(|e| KeychainError::Backend(e.to_string())),
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

#[cfg(all(test, target_os = "macos"))]
mod macos_live_tests {
    use super::*;

    #[test]
    #[ignore = "hits the live login keychain; run manually with --ignored"]
    fn real_keychain_round_trip() {
        let kc = SecurityFrameworkKeychain::new();
        kc.set("agent-bus-app-test", "s1-roundtrip", "sk-live").unwrap();
        assert_eq!(kc.get("agent-bus-app-test", "s1-roundtrip").unwrap(), "sk-live");
        kc.delete("agent-bus-app-test", "s1-roundtrip").unwrap();
        assert!(matches!(kc.get("agent-bus-app-test", "s1-roundtrip"), Err(KeychainError::NotFound)));
    }
}
```

- [ ] **Step 2: Verify it compiles (macOS) and the non-ignored tests still pass**

Run: `cargo test -p secrets`
Expected: 4 tests pass, 1 ignored. If the `security_framework::passwords` API path differs in 3.7, adjust the `use` paths (the functions are `set_generic_password`/`get_generic_password`/`delete_generic_password`); confirm via `cargo check -p secrets`.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/secrets/src/keychain.rs
git commit -m "feat(secrets): macOS SecurityFrameworkKeychain (structural, live test ignored) (S1)"
```

---

## Task 3: Keychain OHS commands (set/status/clear)

**Files:**
- Modify: `src-tauri/secrets/src/api.rs`

- [ ] **Step 1: Write the failing test + the module in `src-tauri/secrets/src/api.rs`**

```rust
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
        description: "Report whether a runner API key is stored in the keychain (presence only).".into(),
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
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p secrets`
Expected: 7 tests pass, 1 ignored.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/secrets/src/api.rs
git commit -m "feat(secrets): set/status/clear API-key OHS commands (write-only secret) (S1)"
```

---

## Task 4: ProjectStore::remove + workspace_remove_project

**Files:**
- Modify: `src-tauri/workspace/src/store.rs`
- Modify: `src-tauri/workspace/src/api.rs`

- [ ] **Step 1: Write the failing store test**

Add to the `tests` mod in `src-tauri/workspace/src/store.rs`:

```rust
    #[tokio::test]
    async fn remove_deletes_the_project() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);
        let p = Project::new("Demo".into(), "/tmp/demo".into(), 100);
        store.insert(&p).await.unwrap();
        store.remove(&p.id).await.unwrap();
        assert!(matches!(store.get(&p.id).await, Err(ProjectStoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn remove_missing_is_not_found() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);
        assert!(matches!(
            store.remove(&ProjectId("nope".into())).await,
            Err(ProjectStoreError::NotFound(_))
        ));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p workspace remove_`
Expected: FAIL — no method `remove`.

- [ ] **Step 3: Implement `ProjectStore::remove`**

Add to `impl ProjectStore` in `src-tauri/workspace/src/store.rs` (after `set_active_pipeline`):

```rust
    /// Remove a project row. Returns NotFound when the id does not exist.
    /// Deletes only the row — on-disk artifacts under the project root are NOT
    /// touched (Workspace owns the registry, not a destructive filesystem wipe).
    pub async fn remove(&self, id: &ProjectId) -> Result<(), ProjectStoreError> {
        let result = sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(&id.0)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(ProjectStoreError::NotFound(id.clone()));
        }
        Ok(())
    }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p workspace remove_`
Expected: 2 tests pass.

- [ ] **Step 5: Add the OHS command + tool spec**

In `src-tauri/workspace/src/api.rs`, after `workspace_set_active_pipeline`:

```rust
#[tauri::command(rename_all = "snake_case")]
pub async fn workspace_remove_project(
    state: tauri::State<'_, WorkspaceState>,
    id: String,
) -> Result<(), String> {
    state.store.remove(&ProjectId(id)).await.map_err(|e| match e {
        ProjectStoreError::NotFound(_) => "not_found".to_string(),
        other => other.to_string(),
    })
}
```

In `tools()`, add (after the `workspace_set_active_pipeline` ToolSpec):

```rust
        ToolSpec {
            name: "workspace_remove_project".into(),
            description: "Remove a project from the workspace registry (does not delete files).".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
            supplier_context: "workspace".into(),
        },
```

- [ ] **Step 6: Run the workspace tests**

Run: `cargo test -p workspace`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/workspace/src/store.rs src-tauri/workspace/src/api.rs
git commit -m "feat(workspace): remove project (store + OHS command) for Settings Projects (S1)"
```

---

## Task 5: Git author config (migration + commands)

**Files:**
- Create: `src-tauri/app/migrations/009_git_config.sql`
- Create: `src-tauri/workspace/src/git_config.rs`
- Modify: `src-tauri/workspace/src/lib.rs`
- Modify: `src-tauri/app/src/lib.rs` (migration registration only — done in Task 7)

- [ ] **Step 1: Write the migration**

`src-tauri/app/migrations/009_git_config.sql`:

```sql
-- S1: Git author identity used for commits workers make in worktrees.
-- Single-row config (id=1), mirrors usage_config. v1.1 persists; the
-- worktree-commit path that consumes it lands later.
CREATE TABLE IF NOT EXISTS git_config (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    author_name  TEXT NOT NULL DEFAULT '',
    author_email TEXT NOT NULL DEFAULT ''
);
INSERT OR IGNORE INTO git_config (id, author_name, author_email) VALUES (1, '', '');
```

- [ ] **Step 2: Write the failing test + module `src-tauri/workspace/src/git_config.rs`**

```rust
//! Git author identity config — the name/email used for commits workers make in
//! worktrees. Single-row store (id=1), mirrors usage_config. Persisted in v1.1;
//! the worktree-commit path that reads it lands in a later item.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GitConfig {
    pub author_name: String,
    pub author_email: String,
}

pub struct GitConfigState {
    pub pool: SqlitePool,
}

pub async fn load_git_config(pool: &SqlitePool) -> GitConfig {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT author_name, author_email FROM git_config WHERE id = 1")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match row {
        Some((author_name, author_email)) => GitConfig { author_name, author_email },
        None => GitConfig::default(),
    }
}

pub async fn set_git_config_inner(
    pool: &SqlitePool,
    author_name: &str,
    author_email: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE git_config SET author_name = ?, author_email = ? WHERE id = 1")
        .bind(author_name)
        .bind(author_email)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn git_config_get(state: tauri::State<'_, GitConfigState>) -> Result<GitConfig, String> {
    Ok(load_git_config(&state.pool).await)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn git_config_set(
    state: tauri::State<'_, GitConfigState>,
    author_name: String,
    author_email: String,
) -> Result<GitConfig, String> {
    set_git_config_inner(&state.pool, &author_name, &author_email).await?;
    Ok(load_git_config(&state.pool).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/009_git_config.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn defaults_to_empty() {
        let pool = fresh_pool().await;
        assert_eq!(load_git_config(&pool).await, GitConfig::default());
    }

    #[tokio::test]
    async fn set_then_load_round_trip() {
        let pool = fresh_pool().await;
        set_git_config_inner(&pool, "Ada Lovelace", "ada@example.com").await.unwrap();
        let cfg = load_git_config(&pool).await;
        assert_eq!(cfg.author_name, "Ada Lovelace");
        assert_eq!(cfg.author_email, "ada@example.com");
    }
}
```

- [ ] **Step 3: Register the module in `src-tauri/workspace/src/lib.rs`**

Add `pub mod git_config;` alongside the other `pub mod` lines.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p workspace git_config`
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/app/migrations/009_git_config.sql src-tauri/workspace/src/git_config.rs src-tauri/workspace/src/lib.rs
git commit -m "feat(workspace): git author config store + commands (migration 009) (S1)"
```

---

## Task 6: Keychain-first key resolver in runner_for

**Files:**
- Modify: `src-tauri/app/src/lib.rs` (runner_for signature + tests)

- [ ] **Step 1: Change `runner_for` to take an injected key resolver**

Replace the `runner_for` fn body in `src-tauri/app/src/lib.rs` with:

```rust
/// Composition-root factory: map a team's resolved RunnerConfig to a concrete
/// Runner. claude-cli is the default and always available. anthropic-api
/// resolves its per-team API key via the injected `resolve_key` closure
/// (keychain-first, then `api_key_env` — built at the root). The resolved key is
/// a plain String passed into AnthropicApiRunner::new — NO keychain/OS type
/// crosses the Runner trait (ACL seal). A team requesting anthropic-api with no
/// resolvable key yields a clear RunnerError (NOT a panic).
fn runner_for(
    config: &pipeline::model::RunnerConfig,
    resolve_key: &dyn Fn(&pipeline::model::RunnerConfig) -> Option<String>,
) -> Result<Arc<dyn Runner>, RunnerError> {
    use agent_bus_core::RunnerKind;
    match config.kind {
        RunnerKind::ClaudeCli => Ok(Arc::new(ClaudeCliRunner::new())),
        RunnerKind::AnthropicApi => {
            let key = resolve_key(config).ok_or_else(|| {
                RunnerError::Other(
                    "anthropic-api runner: no API key found in the keychain or `api_key_env`".into(),
                )
            })?;
            Ok(Arc::new(AnthropicApiRunner::new(key)))
        }
    }
}
```

- [ ] **Step 2: Update the existing `runner_factory_tests` to pass a resolver**

Replace the `runner_factory_tests` mod with:

```rust
#[cfg(test)]
mod runner_factory_tests {
    use super::runner_for;
    use agent_bus_core::{EffortMode, RunnerKind};
    use pipeline::model::RunnerConfig;

    fn cfg(kind: RunnerKind, api_key_env: Option<&str>) -> RunnerConfig {
        RunnerConfig {
            kind,
            model: "claude-opus-4-7".into(),
            effort: EffortMode::Standard,
            api_key_env: api_key_env.map(|s| s.to_string()),
        }
    }

    // A resolver that mimics the root: keychain-first (here, a canned Some for a
    // sentinel account), then env fallback by api_key_env.
    fn env_resolver(c: &RunnerConfig) -> Option<String> {
        c.api_key_env.as_ref().and_then(|n| std::env::var(n).ok())
    }

    #[test]
    fn claude_cli_kind_builds_a_runner() {
        let r = runner_for(&cfg(RunnerKind::ClaudeCli, None), &env_resolver);
        assert!(r.is_ok(), "claude-cli must always build");
    }

    #[test]
    fn anthropic_api_with_resolvable_key_builds_a_runner() {
        std::env::set_var("R1_TEST_KEY_PRESENT", "sk-test-123");
        let r = runner_for(&cfg(RunnerKind::AnthropicApi, Some("R1_TEST_KEY_PRESENT")), &env_resolver);
        std::env::remove_var("R1_TEST_KEY_PRESENT");
        assert!(r.is_ok(), "anthropic-api with a resolvable key must build");
    }

    #[test]
    fn anthropic_api_resolves_via_keychain_first() {
        // A resolver that returns a key WITHOUT any env var set proves the
        // keychain-first path: the factory uses whatever the resolver yields.
        let kc_resolver = |_c: &RunnerConfig| Some("sk-from-keychain".to_string());
        let r = runner_for(&cfg(RunnerKind::AnthropicApi, None), &kc_resolver);
        assert!(r.is_ok(), "a keychain-resolved key must build even with no api_key_env");
    }

    #[test]
    fn anthropic_api_with_no_resolvable_key_is_a_clear_error_not_a_panic() {
        let none_resolver = |_c: &RunnerConfig| None;
        match runner_for(&cfg(RunnerKind::AnthropicApi, None), &none_resolver) {
            Err(runners::output::RunnerError::Other(msg)) => {
                assert!(msg.to_lowercase().contains("api key"), "msg: {msg}");
            }
            Err(other) => panic!("expected Other, got {other:?}"),
            Ok(_) => panic!("expected a clear error, got a runner"),
        }
    }
}
```

- [ ] **Step 3: Update the call site in `spawn_worker_loops`**

`spawn_worker_loops` must accept the keychain and build the resolver. Change its signature to add a parameter:

```rust
    keychain: Option<Arc<dyn secrets::KeychainStore>>,
```

(add to the `#[allow(clippy::too_many_arguments)]` fn signature, last param).

Then inside the `for team` loop, replace the `runner_for(&effective)` match with:

```rust
        let effective = team.effective_runner();
        let resolve_key = |c: &pipeline::model::RunnerConfig| -> Option<String> {
            // Keychain-first: account = api_key_env name if present, else the
            // default "anthropic-api" account. Then fall back to the env var.
            let account = c.api_key_env.as_deref().unwrap_or("anthropic-api");
            if let Some(kc) = keychain.as_ref() {
                if let Ok(k) = kc.get(secrets::api::SERVICE, account) {
                    if !k.is_empty() {
                        return Some(k);
                    }
                }
            }
            c.api_key_env.as_ref().and_then(|n| std::env::var(n).ok())
        };
        let runner: Arc<dyn Runner> = match runner_for(&effective, &resolve_key) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "app: team `{}` runner selection failed ({e}); falling back to claude-cli",
                    team.id
                );
                Arc::new(ClaudeCliRunner::new())
            }
        };
```

- [ ] **Step 4: Update the `spawn_worker_loops` call site**

At the call (~line 1012), add the keychain arg. In Task 7 the root builds `keychain`; for now pass `None` so it compiles, then Task 7 swaps to `Some(keychain.clone())`:

Search for `spawn_worker_loops(handle.clone(), pipe.clone(), ...` and append `, Some(keychain.clone())` (the keychain Arc is created in Task 7 — order Task 7 before re-running, or temporarily pass `None::<Arc<dyn secrets::KeychainStore>>`). To keep this task self-contained, add `secrets = { path = "../secrets" }` to `app/Cargo.toml` `[dependencies]` now.

- [ ] **Step 5: Run the factory tests**

Run: `cargo test -p agent-bus-app runner_factory`
Expected: 4 tests pass. (The full app build may still need Task 7's wiring for the call site; if so, temporarily pass `None` at the call site to compile, then finalize in Task 7.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/lib.rs src-tauri/app/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(app): keychain-first API-key resolver injected into runner_for (S1)"
```

---

## Task 7: Wire keychain + git config at the root; register migration & commands

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Register migration 009 in both lists + bump the user_version test**

In `run_migrations` MIGRATIONS array add:

```rust
        (9, include_str!("../migrations/009_git_config.sql")),
```

In the second migration list (the `MigrationFile`-style list near line ~850) add the matching entry:

```rust
            sql: include_str!("../migrations/009_git_config.sql"),
```

(match the exact struct shape used for 008). Find the test asserting `user_version == 8` and bump it to `9`.

- [ ] **Step 2: Build the keychain in setup() and manage KeychainState + GitConfigState**

In the `.setup(...)` block, after the WorkspaceState is managed (~line 887), add:

```rust
                // Secrets / keychain (S1). Real OS keychain on macOS; an
                // in-memory fake elsewhere keeps the seam usable in any build.
                #[cfg(target_os = "macos")]
                let keychain: Arc<dyn secrets::KeychainStore> =
                    Arc::new(secrets::SecurityFrameworkKeychain::new());
                #[cfg(not(target_os = "macos"))]
                let keychain: Arc<dyn secrets::KeychainStore> =
                    Arc::new(secrets::FakeKeychain::new());
                handle.manage(secrets::api::KeychainState { store: keychain.clone() });

                // Git author config (S1).
                handle.manage(workspace::git_config::GitConfigState { pool: pool.clone() });
```

- [ ] **Step 3: Pass the keychain into spawn_worker_loops**

At the `spawn_worker_loops(...)` call, change the trailing arg added in Task 6 from `None` to `Some(keychain.clone())`.

- [ ] **Step 4: Add the secrets tools to the catalog union**

Where `specs.extend(...)` calls live (~line 943), add:

```rust
                specs.extend(secrets::api::tools());
```

- [ ] **Step 5: Register the new commands in invoke_handler**

Add to `tauri::generate_handler![ ... ]`:

```rust
            workspace::api::workspace_remove_project,
            workspace::git_config::git_config_get,
            workspace::git_config::git_config_set,
            secrets::api::runner_set_api_key,
            secrets::api::runner_get_api_key_status,
            secrets::api::runner_clear_api_key,
```

- [ ] **Step 6: Build the whole workspace**

Run: `cargo check --workspace`
Expected: clean (no errors).

- [ ] **Step 7: Run the full backend test suite**

Run: `cargo test --workspace`
Expected: all green (including the bumped user_version=9 test).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): wire keychain + git config at root; register migration 009 + S1 commands (S1)"
```

---

## Task 8: Frontend IPC wrappers

**Files:**
- Create: `src/ipc/secrets.ts`
- Modify: `src/ipc/workspace.ts`

- [ ] **Step 1: Write `src/ipc/secrets.ts`**

```ts
import { invoke } from "@tauri-apps/api/core";

/** The runner-key id for the anthropic-api runner key (v1.1 scope). */
export const ANTHROPIC_API_KEY_ID = "anthropic-api";

export async function setRunnerApiKey(keyId: string, key: string): Promise<void> {
  await invoke<void>("runner_set_api_key", { key_id: keyId, key });
}

export async function getRunnerApiKeyStatus(keyId: string): Promise<boolean> {
  return await invoke<boolean>("runner_get_api_key_status", { key_id: keyId });
}

export async function clearRunnerApiKey(keyId: string): Promise<void> {
  await invoke<void>("runner_clear_api_key", { key_id: keyId });
}
```

- [ ] **Step 2: Extend `src/ipc/workspace.ts`**

Append:

```ts
export interface GitConfig {
  author_name: string;
  author_email: string;
}

export async function setActivePipeline(
  id: string,
  pipelineId: string | null,
): Promise<void> {
  await invoke<void>("workspace_set_active_pipeline", { id, pipeline_id: pipelineId });
}

export async function removeProject(id: string): Promise<void> {
  await invoke<void>("workspace_remove_project", { id });
}

export async function getGitConfig(): Promise<GitConfig> {
  return await invoke<GitConfig>("git_config_get");
}

export async function setGitConfig(authorName: string, authorEmail: string): Promise<GitConfig> {
  return await invoke<GitConfig>("git_config_set", {
    author_name: authorName,
    author_email: authorEmail,
  });
}
```

- [ ] **Step 3: Typecheck**

Run: `cd /Users/tim/projects/agent-bus-app && bun run build` (or `bunx tsc --noEmit`)
Expected: no type errors from these files (the SettingsView wiring lands in Task 9; build may flag unused exports only if strict — that is fine, they are used in Task 9).

- [ ] **Step 4: Commit**

```bash
git add src/ipc/secrets.ts src/ipc/workspace.ts
git commit -m "feat(ipc): secrets + git-config + remove/set-active-pipeline wrappers (S1)"
```

---

## Task 9: SettingsView — Runners / Git / Projects sections

**Files:**
- Modify: `src/components/SettingsView.tsx`
- Modify: `src/components/SettingsView.test.tsx`
- Modify: `src/App.tsx`

- [ ] **Step 1: Write the failing tests in `src/components/SettingsView.test.tsx`**

Add (keeping the existing tests intact; new props are passed to every `render`). Define a shared `baseProps` helper at the top of the describe and use it. Add tests:

```tsx
  it("shows the runners api-key section with a save control", () => {
    render(<SettingsView {...baseProps()} />);
    expect(screen.getByText(/runners/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/api key/i)).toBeInTheDocument();
  });

  it("saves the api key via onSetApiKey", async () => {
    const onSetApiKey = vi.fn().mockResolvedValue(undefined);
    render(<SettingsView {...baseProps({ onSetApiKey })} />);
    fireEvent.change(screen.getByLabelText(/api key/i), { target: { value: "sk-xyz" } });
    fireEvent.click(screen.getByRole("button", { name: /save key/i }));
    await waitFor(() => expect(onSetApiKey).toHaveBeenCalledWith("sk-xyz"));
  });

  it("shows the git author section", () => {
    render(<SettingsView {...baseProps()} />);
    expect(screen.getByText(/git/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/author name/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/author email/i)).toBeInTheDocument();
  });

  it("lists projects and exposes remove", () => {
    render(<SettingsView {...baseProps({ projects: [{ id: "p1", name: "Alpha", root_path: "/a", active_pipeline_id: null, created_at: 0, updated_at: 0 }] })} />);
    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /remove/i })).toBeInTheDocument();
  });
```

Add a `baseProps` helper (merge in overrides):

```tsx
  function baseProps(over: Partial<React.ComponentProps<typeof SettingsView>> = {}) {
    return {
      usage: snap,
      onSetBudget: vi.fn().mockResolvedValue(snap),
      onSetAutoMeter: vi.fn().mockResolvedValue(snap),
      apiKeyPresent: false,
      onSetApiKey: vi.fn().mockResolvedValue(undefined),
      onClearApiKey: vi.fn().mockResolvedValue(undefined),
      gitConfig: { author_name: "", author_email: "" },
      onSaveGitConfig: vi.fn().mockResolvedValue({ author_name: "", author_email: "" }),
      projects: [],
      activeProjectId: null,
      onRemoveProject: vi.fn().mockResolvedValue(undefined),
      ...over,
    } as React.ComponentProps<typeof SettingsView>;
  }
```

Ensure imports include `fireEvent`, `waitFor` from `@testing-library/react` and that `snap` is the existing fixture.

- [ ] **Step 2: Run to verify they fail**

Run: `bun vitest run src/components/SettingsView.test.tsx`
Expected: FAIL — new props/sections not present.

- [ ] **Step 3: Rewrite `src/components/SettingsView.tsx`**

```tsx
import { useState, type CSSProperties } from "react";
import type { UsageSnapshot } from "../ipc/usage";
import type { GitConfig, Project } from "../ipc/workspace";
import { Button } from "./ui/Button";
import { formatTokens } from "../lib/cost";

export interface SettingsViewProps {
  usage: UsageSnapshot | null;
  onSetBudget: (budget: number) => Promise<UsageSnapshot>;
  onSetAutoMeter: (enabled: boolean) => Promise<UsageSnapshot>;
  // Runners (R1 API key via keychain)
  apiKeyPresent: boolean;
  onSetApiKey: (key: string) => Promise<void>;
  onClearApiKey: () => Promise<void>;
  // Git author
  gitConfig: GitConfig;
  onSaveGitConfig: (name: string, email: string) => Promise<GitConfig>;
  // Projects
  projects: Project[];
  activeProjectId: string | null;
  onRemoveProject: (id: string) => Promise<void>;
}

type Theme = "dark" | "light";

function currentTheme(): Theme {
  return (document.documentElement.getAttribute("data-theme") as Theme) ?? "dark";
}

export function SettingsView(props: SettingsViewProps) {
  const {
    usage, onSetBudget, onSetAutoMeter,
    apiKeyPresent, onSetApiKey, onClearApiKey,
    gitConfig, onSaveGitConfig,
    projects, activeProjectId, onRemoveProject,
  } = props;

  const [theme, setTheme] = useState<Theme>(currentTheme());
  const [budgetInput, setBudgetInput] = useState(String(usage?.window_budget ?? 2_600_000));
  const [saving, setSaving] = useState(false);
  const [autoMeter, setAutoMeter] = useState<boolean>(usage?.auto_meter_enabled ?? false);
  const [autoSaving, setAutoSaving] = useState(false);

  const [apiKeyInput, setApiKeyInput] = useState("");
  const [keyPresent, setKeyPresent] = useState(apiKeyPresent);
  const [keySaving, setKeySaving] = useState(false);

  const [gitName, setGitName] = useState(gitConfig.author_name);
  const [gitEmail, setGitEmail] = useState(gitConfig.author_email);
  const [gitSaving, setGitSaving] = useState(false);

  async function toggleAutoMeter(next: boolean) {
    setAutoMeter(next);
    setAutoSaving(true);
    try {
      const s = await onSetAutoMeter(next);
      setAutoMeter(s.auto_meter_enabled);
    } finally {
      setAutoSaving(false);
    }
  }

  function applyTheme(next: Theme) {
    document.documentElement.setAttribute("data-theme", next);
    setTheme(next);
  }

  async function saveBudget() {
    const n = Number(budgetInput);
    if (!Number.isFinite(n) || n <= 0) return;
    setSaving(true);
    try {
      await onSetBudget(n);
    } finally {
      setSaving(false);
    }
  }

  async function saveApiKey() {
    if (apiKeyInput.trim() === "") return;
    setKeySaving(true);
    try {
      await onSetApiKey(apiKeyInput);
      setApiKeyInput("");
      setKeyPresent(true);
    } finally {
      setKeySaving(false);
    }
  }

  async function clearApiKey() {
    setKeySaving(true);
    try {
      await onClearApiKey();
      setKeyPresent(false);
    } finally {
      setKeySaving(false);
    }
  }

  async function saveGit() {
    setGitSaving(true);
    try {
      const c = await onSaveGitConfig(gitName, gitEmail);
      setGitName(c.author_name);
      setGitEmail(c.author_email);
    } finally {
      setGitSaving(false);
    }
  }

  const section: CSSProperties = { maxWidth: 560, marginBottom: "var(--sp-10)" };
  const h: CSSProperties = { fontSize: 11, color: "var(--text-3)", textTransform: "lowercase", letterSpacing: "0.04em", marginBottom: "var(--sp-3)", borderBottom: "1px solid var(--border)", paddingBottom: 4 };
  const label: CSSProperties = { fontSize: 12, color: "var(--text-2)", display: "block", marginBottom: 6 };
  const input: CSSProperties = { background: "var(--surface)", border: "1px solid var(--border)", color: "var(--text)", borderRadius: "var(--r-sm)", padding: "5px 10px", fontSize: 12, fontFamily: "inherit", width: 260 };

  return (
    <div style={{ padding: "var(--sp-8)" }}>
      <div style={section}>
        <div style={h}>general</div>
        <span style={label}>theme</span>
        <div style={{ display: "flex", gap: 8 }}>
          <Button variant={theme === "dark" ? "primary" : "default"} onClick={() => applyTheme("dark")}>dark</Button>
          <Button variant={theme === "light" ? "primary" : "default"} onClick={() => applyTheme("light")}>light</Button>
        </div>
      </div>

      <div style={section}>
        <div style={h}>usage</div>
        <label style={label} htmlFor="budget">window budget (tokens / 5h)</label>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <input id="budget" value={budgetInput} onChange={(e) => setBudgetInput(e.target.value)}
            style={{ ...input, width: 160, fontVariantNumeric: "tabular-nums" }} />
          <Button variant="primary" disabled={saving} onClick={saveBudget}>save budget</Button>
        </div>
        {usage && (
          <div style={{ marginTop: 8, fontSize: 11, color: "var(--text-3)" }}>
            currently {formatTokens(usage.window_total)} of {formatTokens(usage.window_budget)} used this window.
          </div>
        )}
        <label style={{ ...label, display: "flex", alignItems: "center", gap: 8, marginTop: 16, marginBottom: 0, cursor: "pointer" }}>
          <input type="checkbox" checked={autoMeter} disabled={autoSaving}
            onChange={(e) => toggleAutoMeter(e.target.checked)} aria-label="auto-brake"
            style={{ accentColor: "var(--accent)", cursor: "pointer" }} />
          auto-brake when the window crosses the threshold
        </label>
        <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-3)" }}>
          off by default — manual + reactive (rate-limit) braking stays on either way.
        </div>
      </div>

      <div style={section}>
        <div style={h}>runners</div>
        <label style={label} htmlFor="api-key">anthropic api key (stored in the OS keychain)</label>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <input id="api-key" type="password" value={apiKeyInput} aria-label="api key"
            placeholder={keyPresent ? "•••••••• (stored)" : "sk-ant-..."}
            onChange={(e) => setApiKeyInput(e.target.value)} style={input} />
          <Button variant="primary" disabled={keySaving} onClick={saveApiKey}>save key</Button>
          {keyPresent && <Button disabled={keySaving} onClick={clearApiKey}>clear</Button>}
        </div>
        <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-3)" }}>
          {keyPresent ? "a key is stored in the keychain." : "no key stored — the anthropic-api runner falls back to the api_key_env var."}
        </div>
      </div>

      <div style={section}>
        <div style={h}>git</div>
        <label style={label} htmlFor="git-name">author name</label>
        <input id="git-name" aria-label="author name" value={gitName}
          onChange={(e) => setGitName(e.target.value)} style={input} />
        <label style={{ ...label, marginTop: 10 }} htmlFor="git-email">author email</label>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <input id="git-email" aria-label="author email" value={gitEmail}
            onChange={(e) => setGitEmail(e.target.value)} style={input} />
          <Button variant="primary" disabled={gitSaving} onClick={saveGit}>save</Button>
        </div>
        <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-3)" }}>
          used for commits workers make in worktrees (applied when worker-commits land).
        </div>
      </div>

      <div style={section}>
        <div style={h}>projects</div>
        {projects.length === 0 && (
          <div style={{ fontSize: 11, color: "var(--text-3)" }}>no projects yet.</div>
        )}
        {projects.map((p) => (
          <div key={p.id} style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 0", borderBottom: "1px solid var(--border)" }}>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 12, color: "var(--text)" }}>
                {p.name}{p.id === activeProjectId ? " (active)" : ""}
              </div>
              <div style={{ fontSize: 11, color: "var(--text-3)" }}>{p.root_path}</div>
            </div>
            <Button onClick={() => onRemoveProject(p.id)}>remove</Button>
          </div>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run the SettingsView tests**

Run: `bun vitest run src/components/SettingsView.test.tsx`
Expected: all pass (old + new).

- [ ] **Step 5: Wire `App.tsx`**

In `src/App.tsx`:
- Import the new IPC: `import { removeProject, getGitConfig, setGitConfig, type GitConfig } from "./ipc/workspace";` and `import { setRunnerApiKey, clearRunnerApiKey, getRunnerApiKeyStatus, ANTHROPIC_API_KEY_ID } from "./ipc/secrets";`
- Add state + an effect that loads `apiKeyPresent` and `gitConfig` on mount:

```tsx
  const [apiKeyPresent, setApiKeyPresent] = useState(false);
  const [gitConfig, setGitConfigState] = useState<GitConfig>({ author_name: "", author_email: "" });
  useEffect(() => {
    getRunnerApiKeyStatus(ANTHROPIC_API_KEY_ID).then(setApiKeyPresent).catch(() => {});
    getGitConfig().then(setGitConfigState).catch(() => {});
  }, []);
```

- Replace the `<SettingsView .../>` usage with:

```tsx
          <SettingsView
            usage={usage}
            onSetBudget={setBudget}
            onSetAutoMeter={setAutoMeter}
            apiKeyPresent={apiKeyPresent}
            onSetApiKey={async (k) => { await setRunnerApiKey(ANTHROPIC_API_KEY_ID, k); setApiKeyPresent(true); }}
            onClearApiKey={async () => { await clearRunnerApiKey(ANTHROPIC_API_KEY_ID); setApiKeyPresent(false); }}
            gitConfig={gitConfig}
            onSaveGitConfig={async (n, e) => { const c = await setGitConfig(n, e); setGitConfigState(c); return c; }}
            projects={projects}
            activeProjectId={activeProject?.id ?? null}
            onRemoveProject={async (id) => { await removeProject(id); await reload(); }}
          />
```

- [ ] **Step 6: Typecheck + build**

Run: `cd /Users/tim/projects/agent-bus-app && bun run build`
Expected: clean build.

- [ ] **Step 7: Commit**

```bash
git add src/components/SettingsView.tsx src/components/SettingsView.test.tsx src/App.tsx
git commit -m "feat(settings): Runners (keychain key) / Git author / Projects sections (S1)"
```

---

## Task 10: Full verification + merge + backlog

- [ ] **Step 1: Full backend verification**

```bash
cargo test --workspace
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: all green; clippy clean (fix any lints, e.g. needless closures, `Default` impls).

- [ ] **Step 2: Full frontend verification**

```bash
cd /Users/tim/projects/agent-bus-app && bun vitest run && bun run build
```
Expected: all vitest pass, build succeeds.

- [ ] **Step 3: Merge --no-ff into main + tag (do NOT push)**

```bash
cd /Users/tim/projects/agent-bus-app
git checkout main
git merge --no-ff plan-s1-settings-keychain -m "Merge plan-s1-settings-keychain: S1 full Settings sections + keychain"
git tag plan-s1-settings-keychain
```

- [ ] **Step 4: Update the backlog**

In `docs/v1.1-backlog.md`, change the S1 line from `[ ]` / `backlog` to `[x]` / `done` with the tag and a one-line summary of what shipped (sections + keychain seam + key resolution). Commit:

```bash
git add docs/v1.1-backlog.md
git commit -m "docs(s1): mark S1 done in v1.1 backlog (tag plan-s1-settings-keychain)"
```

---

## Self-Review

- **Spec coverage:** Runners section (keychain key) → Tasks 1–3, 6, 9. Git section → Task 5, 9. Projects section (list/switch/remove) → Task 4, 9 (switch via existing `set_active_pipeline`; the Projects section lists + removes; active is shown). Keychain trait + fake + real → Tasks 1–2. Key resolution keychain-first + env fallback → Task 6. No plaintext to SQLite → keychain holds the secret; only a presence bool + git author land in SQLite. ✓
- **Placeholders:** none — all code is concrete.
- **Type consistency:** `KeychainStore`/`FakeKeychain`/`SecurityFrameworkKeychain`, `KeychainState.store`, `GitConfig {author_name, author_email}`, resolver signature `&dyn Fn(&RunnerConfig)->Option<String>`, IPC `setRunnerApiKey/getRunnerApiKeyStatus/clearRunnerApiKey`, `SettingsViewProps` all consistent across tasks. ✓
- **Note:** Task 6/7 ordering — Task 6 introduces a `spawn_worker_loops` arg; to keep each task compiling, pass `None` at the call site in Task 6 and switch to `Some(keychain.clone())` in Task 7. Acceptable since both are committed before verification in Task 10.
