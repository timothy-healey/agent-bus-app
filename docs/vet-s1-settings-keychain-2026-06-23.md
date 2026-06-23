---
id: vet-s1-settings-keychain-2026-06-23
verb: vet
target: plans/2026-06-23-plan-s1-settings-keychain.md
date: 2026-06-23
lens: strategic · vet · brief
verdict: SOUND WITH FIXES
---

# Vet — S1 Full Settings sections + keychain

Reviewed: `plans/2026-06-23-plan-s1-settings-keychain.md` against `DOMAIN.md` and the
affected code (`src-tauri/runners/src/anthropic_api.rs`, `app/src/lib.rs::runner_for`,
`workspace/src/{api,store}.rs`, `usage_telemetry/src/api.rs`, `src/components/SettingsView.tsx`).

## Summary

The plan is DDD-sound on its load-bearing claims: the keychain is a dedicated infrastructure
concern behind a trait wired at the composition root (DD1), the api-key read path keeps the
Runners ACL sealed because the resolver yields a plain `String` and `AnthropicApiRunner::new`
already takes a `String` (DD2 — verified against `anthropic_api.rs:145`), Projects settings
reuse Workspace's existing OHS commands plus an additive `remove` (DD4), and no plaintext
secret is persisted to SQLite/disk (only a presence bool + git author land in the DB). Five
findings, none structural; all are language/seam-hygiene amendments applied in-plan.

## Findings

### F1 [medium] §E-boundary — `secrets` is an eighth module; name it as infrastructure, not a bounded context

**What:** The plan adds a new `secrets` crate (DD1, File Structure). `DOMAIN.md` declares
*seven* bounded contexts; a casual reader could mistake `secrets` for an eighth context with
its own ubiquitous language. It is not — it is a **generic-subdomain infrastructure seam** (a
secure key-value store), the same architectural tier as `agent_bus_core`: owned by the
architecture, depended on at the root, depending on nothing project-internal.

**Cited plan section:** DD1; "File Structure" (`src-tauri/secrets/`); Task 1.

**Why it matters:** Unnamed tiers drift. If `secrets` reads as a context, future work will
pile domain logic into it (e.g. per-team key *policy*). Naming it a generic subdomain keeps
it thin — store/get/delete a secret, nothing more — and keeps the seven-context map honest.

**Amendment (applied):** DD1 states explicitly that `secrets` is a **generic-subdomain
infrastructure crate** (peer tier to `agent_bus_core`, not an eighth bounded context); the
module doc-comment in `keychain.rs` says the same; v1.1 scope is fixed to API-key storage.
Register the concept in `DOMAIN.md` after merge (see F5). **Status:** resolved.

### F2 [medium] §E-language — "account" is an OS-keychain idiom leaking into the app vocabulary

**What:** The keychain commands key secrets by `account` (`runner_set_api_key(account, key)`,
`SERVICE`/`account` in `keychain.rs`). `account` is the macOS Security-framework term
(`kSecAttrAccount`). At the OHS surface and in the frontend (`ANTHROPIC_API_ACCOUNT`), the
app concept is "**which runner key**", not an OS account.

**Cited plan section:** Task 3 (`runner_set_api_key(account,...)`); Task 8 (`ANTHROPIC_API_ACCOUNT`).

**Why it matters:** A generic-subdomain seam must speak the *consumer's* language at its
boundary and seal the OS idiom inside — exactly the ACL discipline DOMAIN.md applies to
Runners. `account` is fine *inside* `keychain.rs` (it is the OS API's word); it should not be
the word the OHS/UI uses.

**Amendment (applied):** Keep `account` only inside the `KeychainStore` impls (it is the OS
term there). At the OHS + IPC surface, the parameter is named `key_id` (the runner-key
identifier; the default value stays the string `"anthropic-api"`). The keychain commands map
`key_id` → the OS `account` slot internally. The `KeychainStore` trait still takes
`(service, account)` — that trait is the OS-facing seam and may keep the OS word.
**Status:** resolved.

### F3 [low] §E-add-over-refactor — confirm `runner_for`'s resolver replaces, not duplicates, the env-read path

**What:** Today `runner_for` reads the env var directly (`app/src/lib.rs:1106`). The plan
injects a `resolve_key` closure (DD2, Task 6). Good — but the plan must ensure the OLD
direct-env branch is *removed*, not left beside the new resolver, or two key-resolution paths
will coexist (the §E "add where a refactor fits" smell).

**Cited plan section:** Task 6, Step 1 (new `runner_for`) + Step 3 (resolver in `spawn_worker_loops`).

**Why it matters:** Refactor-before-add. One key-resolution policy, one place.

**Amendment (applied):** Task 6 Step 1 fully *replaces* the `runner_for` body (the env read
now lives only inside the root-built resolver in Step 3); the old `std::env::var(env_name)`
branch is deleted from `runner_for`. The env fallback exists in exactly one place (the
resolver). **Status:** resolved.

### F4 [low] §E-language — "presence bool" seam is right; name the no-secret-egress invariant on the command

**What:** DD6 / Task 3 return only `has_key: bool` from `runner_get_api_key_status`, never the
secret. This is the correct invariant (no plaintext egress to the frontend), but it is stated
in prose, not pinned as an invariant the command owns.

**Cited plan section:** DD6; Task 3 (`api_key_status_inner` → bool; `runner_get_api_key_status`).

**Why it matters:** The invariant "the secret never crosses back to the UI" is the whole point
of the keychain over a SQLite column. An invariant worth having is worth naming where it is
protected, so a future `get_api_key` convenience command is recognised as a violation.

**Amendment (applied):** The `api.rs` module doc-comment names the invariant: *"the secret is
write-only across the boundary — `set`/`clear` cross it, status returns presence only; no
command ever returns the stored secret."* The `status_only_returns_presence_never_the_secret`
test stands as its guard. **Status:** resolved.

### F5 [low] §E-language — register the new ubiquitous-language terms in DOMAIN.md

**What:** S1 introduces durable concepts absent from `DOMAIN.md`: the **Keychain / secret
store** seam, the **runner key** identifier, and **Git author identity** config.

**Cited plan section:** whole plan; esp. DD1, DD3, DD6.

**Why it matters:** "The language lives in the code" — and in DOMAIN.md. Terms that ship
without registration become tribal knowledge.

**Amendment (applied):** After merge, add to `DOMAIN.md`: under a new **Generic subdomains**
note — *Keychain / Secret store* (the `KeychainStore` seam; real macOS impl + fake; stores the
runner API key; secret never persisted to SQLite/disk, never returned across the OHS); and
under **Workspace** — *Git author identity* (`git_config` single-row store; the name/email for
worker worktree commits; persisted in v1.1, consumed when worker-commits land). Applied
to `DOMAIN.md` in this branch (new **Generic subdomains (infrastructure)** section +
**Workspace → Git author identity** entry). **Status:** resolved.

## Verdict

**SOUND WITH FIXES.** No structural redesign required. F1/F2 keep the new generic-subdomain
seam honest (named as infrastructure; OS idiom sealed at the boundary); F3 enforces
refactor-before-add on the resolver; F4 pins the no-secret-egress invariant; F5 registers the
new language. All five are amendments to the plan/doc, applied. Proceed to implementation.
