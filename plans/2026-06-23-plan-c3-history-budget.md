# C3 — Accurate History-Budget Accounting for Tool-Call Turns — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `Turn::estimated_tokens` account for tool-call **args** and **results** so the Conversation aggregate's history-budget invariant ("total history tokens ≤ `history_budget_tokens`, drop oldest pair") no longer systematically under-counts fat agentic composite turns.

**Architecture:** Single-file, aggregate-internal change to `conversational_control/src/turn.rs`. Replace the per-tool-call constant `tool_name.len()/4 + 8` with a coarse `serde_json`-length-based estimate that folds in the serialized size of `request.args` and the `result` (the `Ok { result }` value or the `Err { error }` string). The estimate stays deterministic (no model call) so the existing budget tests remain stable. No new dependencies, no new public API, no edge into any supplier context — the kernel stays kernel-only.

**Tech Stack:** Rust (cargo workspace under `src-tauri/`), `serde`/`serde_json` (already a dependency of `agent_bus_core` and used in `turn.rs` tests), `cargo test`.

---

## Decisions

Resolved ambiguities (recommended option chosen for each — operator is AFK):

- **D1 — Estimation algorithm.** Use the vet's recommended "coarse `serde_json` length / 4 per call". Concretely, per tool-call charge: `tool_name.chars().count()/4` + `serde_json::to_string(&request.args).len()/4` + result size, where result size is `serde_json::to_string(result_value).len()/4` for `Ok`, `error.chars().count()/4` for `Err`, and `0` for `None` (dispatched-but-unresolved). Keep the existing `+ 8` per-call structural constant (covers JSON envelope/role framing overhead) so the estimate only ever grows relative to v1 — never shrinks — which is the safe direction for a budget invariant.
- **D2 — `chars().count()` vs `len()` for JSON.** The existing code uses `chars().count()` for the (human) `text` and `tool_name`. For serialized JSON args/results we use byte `len()`: serialized JSON is ASCII-dominated and `len()` is an upper bound on `chars().count()`, which biases the estimate slightly conservative (toward over-counting) — the safe direction for a budget. Keep `chars().count()` for `tool_name` to match the existing call exactly and avoid changing that sub-term's behavior.
- **D3 — Helper extraction.** Extract a private `fn tool_call_tokens(tc: &ToolCall) -> usize` so the per-call estimate is one readable, testable unit and `estimated_tokens` stays a clean sum. Keeps DRY and makes the new behavior directly unit-testable.
- **D4 — Serialization fallibility.** `serde_json::to_string` returns `Result`. The values here are already-deserialized `serde_json::Value`s, which always re-serialize, but to stay panic-free we use `.map(|s| s.len()).unwrap_or(0)` rather than `.unwrap()`. Falling back to `0` on the impossible error path keeps the estimate from panicking inside budget math.
- **D5 — Scope guard.** No change to `conversation.rs` (`truncate_to_budget`/`Conversation::estimated_tokens` already delegate to `Turn::estimated_tokens` — they automatically benefit). No change to `agent_bus_core` types. No public-API change to `Turn`. This keeps the change aggregate-internal and the kernel acyclic, exactly as the C1/composite-engine vet required.

---

## File Structure

- **Modify:** `src-tauri/conversational_control/src/turn.rs`
  - Rewrite `Turn::estimated_tokens` to sum a new private helper `tool_call_tokens` over `tool_calls`.
  - Add private `fn tool_call_tokens(tc: &ToolCall) -> usize` (free function in the module, or associated fn — see Task code).
  - Add unit tests covering: args contribute, results contribute, `Err` contributes, `None` result contributes nothing beyond the constant, and a fat composite turn out-weighs a bare-name turn.
  - Update the doc-comment on `estimated_tokens` to state it now folds in args + result size.

No other files change. `conversation.rs` budget tests stay green because the estimate only grows (the tiny-budget truncation test at `conversation.rs:118` uses 200-char text turns with no tool-calls, so its numbers are unaffected).

---

## Task 1: Fold tool-call args + result size into the per-turn token estimate

**Files:**
- Modify: `src-tauri/conversational_control/src/turn.rs` (`estimated_tokens` at lines 42-52; tests module from line 55)
- Test: same file, `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing tests**

Add these tests to the `mod tests` block in `src-tauri/conversational_control/src/turn.rs` (after the existing `estimated_tokens_grows_with_text_and_tool_calls`):

```rust
    #[test]
    fn estimated_tokens_counts_tool_call_args() {
        let small = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "t".into(), args: json!({}) },
                result: None,
            }],
            0,
        );
        let fat_args = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest {
                    tool_name: "t".into(),
                    args: json!({ "blob": "x".repeat(400) }),
                },
                result: None,
            }],
            0,
        );
        // ~400 chars of args => ~100 extra tokens; must clearly exceed the small call.
        assert!(
            fat_args.estimated_tokens() > small.estimated_tokens() + 50,
            "fat args ({}) should dwarf empty args ({})",
            fat_args.estimated_tokens(),
            small.estimated_tokens()
        );
    }

    #[test]
    fn estimated_tokens_counts_tool_call_result() {
        let no_result = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "t".into(), args: json!({}) },
                result: None,
            }],
            0,
        );
        let fat_result = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "t".into(), args: json!({}) },
                result: Some(ToolCallResult::Ok { result: json!({ "out": "y".repeat(400) }) }),
            }],
            0,
        );
        assert!(
            fat_result.estimated_tokens() > no_result.estimated_tokens() + 50,
            "fat result ({}) should dwarf no result ({})",
            fat_result.estimated_tokens(),
            no_result.estimated_tokens()
        );
    }

    #[test]
    fn estimated_tokens_counts_err_result_message() {
        let no_result = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "t".into(), args: json!({}) },
                result: None,
            }],
            0,
        );
        let fat_err = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "t".into(), args: json!({}) },
                result: Some(ToolCallResult::Err { error: "e".repeat(400) }),
            }],
            0,
        );
        assert!(
            fat_err.estimated_tokens() > no_result.estimated_tokens() + 50,
            "fat err ({}) should dwarf no result ({})",
            fat_err.estimated_tokens(),
            no_result.estimated_tokens()
        );
    }

    #[test]
    fn estimated_tokens_unresolved_call_charges_only_structural_constant() {
        // A None result must not add result-size tokens; only the per-call
        // constant + name + (empty) args. Guards against the estimate ballooning
        // on dispatched-but-unresolved calls.
        let t = Turn::assistant(
            "",
            vec![ToolCall {
                request: ToolCallRequest { tool_name: "approve_gate".into(), args: json!({}) },
                result: None,
            }],
            0,
        );
        // name (12 chars /4 = 3) + args "{}" (2 bytes /4 = 0) + 8 constant + 4 turn base = 15.
        assert_eq!(t.estimated_tokens(), 15);
    }
```

Also import `ToolCallResult` in the test module — change the existing `use super::*;` block: the tests already `use serde_json::json;` and `super::*` re-exports `ToolCall`/`ToolCallRequest` via the module's top `use`. Confirm `ToolCallResult` is in scope (it is imported at the top of the file: `use agent_bus_core::{ToolCallRequest, ToolCallResult};`, and `use super::*;` brings it in). No extra `use` needed.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p conversational_control turn:: 2>&1 | tail -30`
Expected: the three "fat" tests FAIL (current estimate ignores args/result, so fat ≈ small) and `estimated_tokens_unresolved_call_charges_only_structural_constant` FAILS on the exact value (current formula yields `name 12/4=3 + 8 + 4 = 15` for the constant path — actually equal; if it already passes that is fine, it locks behavior). The args/result tests are the load-bearing failures.

- [ ] **Step 3: Write the implementation**

Replace the `estimated_tokens` method (lines 42-52) with the following, and add the helper. New body of the `impl Turn` block's estimate section:

```rust
    /// Rough token estimate for history-budget math (D: ~4 chars/token, plus a
    /// small per-tool-call constant). Folds in each embedded tool-call's args
    /// and result payload so fat agentic composite turns are counted accurately
    /// (C3). Deterministic so budget tests are stable.
    pub fn estimated_tokens(&self) -> usize {
        let text = self.text.chars().count() / 4;
        let tools: usize = self.tool_calls.iter().map(tool_call_tokens).sum();
        text + tools + 4
    }
```

Add this free function inside `turn.rs`, after the `impl Turn { ... }` block (before `#[cfg(test)]`):

```rust
/// Coarse token estimate for one embedded tool-call: the tool name, the
/// serialized args, and the serialized result payload (the Ok value or the Err
/// message), plus a small structural constant for the call's JSON envelope.
/// Byte length is used for serialized JSON (an upper bound on char count), which
/// biases the estimate conservatively — the safe direction for a budget cap.
fn tool_call_tokens(tc: &ToolCall) -> usize {
    let name = tc.request.tool_name.chars().count() / 4;
    let args = serde_json::to_string(&tc.request.args)
        .map(|s| s.len())
        .unwrap_or(0)
        / 4;
    let result = match &tc.result {
        Some(ToolCallResult::Ok { result }) => {
            serde_json::to_string(result).map(|s| s.len()).unwrap_or(0) / 4
        }
        Some(ToolCallResult::Err { error }) => error.chars().count() / 4,
        None => 0,
    };
    name + args + result + 8
}
```

Note: `serde_json` must be usable in non-test code in this crate. Verify Step 4's compile; if `serde_json` is only a dev-dependency, see the fallback below.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p conversational_control 2>&1 | tail -30`
Expected: PASS — all turn tests (new + existing) and all conversation tests green.

If compilation fails with `use of undeclared crate or module serde_json`, then `serde_json` is dev-only in this crate's `Cargo.toml`. Fallback: promote it to a normal dependency:

Run: `cd src-tauri && cargo add serde_json -p conversational_control`
Then re-run Step 4. (Check first with: `grep -n serde_json src-tauri/conversational_control/Cargo.toml` — if it appears under `[dependencies]` already, no action needed.)

- [ ] **Step 5: Verify the whole workspace still builds and is clippy-clean**

Run: `cd src-tauri && cargo test --workspace 2>&1 | tail -20 && cargo clippy --workspace --all-targets 2>&1 | tail -20`
Expected: all tests pass; clippy reports no warnings. If clippy flags `map(...).unwrap_or(0)` as `map_or`, apply its suggestion (`serde_json::to_string(...).map_or(0, |s| s.len())`).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/conversational_control/src/turn.rs src-tauri/conversational_control/Cargo.toml
git commit -m "fix(conversational_control): count tool-call args + results in history-budget estimate

Turn::estimated_tokens charged only name.len()/4 + 8 per tool-call,
ignoring request.args and result payloads. Fat agentic composite turns
(C1) therefore under-counted the Conversation history-budget invariant,
letting the real context window grow past history_budget_tokens. Fold a
coarse serde_json length/4 for args and result (Ok value / Err message)
into the per-call estimate via a new tool_call_tokens helper.

Closes C3 (composite-engine vet F3)."
```

---

## Self-Review

**1. Spec coverage.** Item C3 = "Fix the estimate to account for args + result size." Task 1 rewrites `estimated_tokens` to fold in `request.args`, `Ok.result`, and `Err.error`. Vet F3's "coarse serde_json length / 4 per call" recommendation is implemented verbatim. Covered.

**2. Placeholder scan.** No TBD/TODO/"handle edge cases" — every step has concrete code and exact commands. The only conditional ("if serde_json is dev-only") is a guarded, fully-specified fallback with the exact command, not a placeholder.

**3. Type consistency.** Helper named `tool_call_tokens` in both the method body (`.map(tool_call_tokens)`) and its definition. Uses `ToolCall`, `ToolCallRequest`, `ToolCallResult` exactly as defined in `agent_bus_core::tool_protocol` (`Ok { result }`, `Err { error }`) and `turn.rs` (`ToolCall { request, result: Option<ToolCallResult> }`). `request.tool_name` / `request.args` match `ToolCallRequest`. Consistent.
