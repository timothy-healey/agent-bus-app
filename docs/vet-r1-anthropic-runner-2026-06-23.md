---
id: vet-r1-anthropic-runner-2026-06-23
verb: vet
date: 2026-06-23
target: plans/2026-06-23-plan-r1-anthropic-runner.md
lens: strategic · vet · brief
verdict: SOUND — 2 doc-clarity findings, both applied
---

# Vet — R1 `anthropic-api` runner

Reviewed the plan against `DOMAIN.md` and the affected code (`runners/src/{output.rs, claude_cli.rs, command.rs, stream_json.rs, api.rs}`, `runtime/src/pool.rs`, `runtime/Cargo.toml`, `app/src/lib.rs`, `pipeline/src/model.rs`). DDD soundness only; decomposition/testability are upstream and out of scope.

## Focus questions (all SOUND)

**(1) Does `AnthropicApiRunner` seal the API idiom?** Yes. The plan confines `reqwest` and `serde_json::Value` to `anthropic_api.rs`. The `Runner` trait + `InvocationRequest`/`RunnerOutput`/`RunnerError`/`RunnerUsage` in `output.rs` mention no anthropic/HTTP/SDK type today (verified: `output.rs` imports only `agent_bus_core::Verdict`, `async_trait`, `serde`, `thiserror`), and the plan adds none — the `SendFn` seam (`Box<dyn Fn(&Value) -> Result<String, RunnerError>>`) keeps the wire `Value` and the `reqwest::blocking` POST *inside* the module, exactly as `claude_cli.rs`'s `SpawnFn` keeps argv + stream-json inside its module and `command.rs`/`stream_json.rs`. The response is translated to `RunnerOutput` before it crosses back. Task 5 Step 3 bakes the seal in as an executable check (`grep -n "reqwest\|serde_json::Value\|anthropic" runners/src/output.rs` → expect no matches). This is a textbook ACL: the slipperiness of Claude's API shape is absorbed at the boundary so Runtime's idiom stays clean (DOMAIN.md cross-expert tension note — Engineer vs AI Engineer — resolves here).

**(2) Is the runner-kind factory at the composition root, with no new edge into Runtime?** Yes. `runner_for(&RunnerConfig) -> Result<Arc<dyn Runner>, RunnerError>` lives in `app/src/lib.rs` and is consumed only by `spawn_worker_loops`. Runtime's dependency surface is unchanged: `runtime/src/pool.rs` imports only `runners::output::{InvocationRequest, LogSink, Runner, RunnerError, RunnerOutput, RunnerUsage}` and `runners::scope` — never a concrete runner, never `runners::anthropic_api`. The pool keeps holding an opaque `Arc<dyn Runner>` (`PoolContext.runner`) and never learns which kind it got. Runner selection is correctly framed as a *composition* concern, not a Runtime or operator concern — consistent with `runners/api.rs` deliberately publishing zero app-tools ("runner selection is a composition concern, not an operator action"). Per-team selection reads `team.effective_runner()` (R5's resolved `RunnerConfig`), so the Authoring↔Runtime shared kernel is respected: Runtime consumes the already-resolved config and the factory reads `kind`/`api_key_env` off it.

**(3) Does the naming align with DOMAIN.md?** Yes, verbatim. DOMAIN.md → Runners (ACL): *"Invocation — one Claude call: CLI subprocess or **API request**"* and *"Runner kind — `claude-cli` (subscription) or `anthropic-api` (key)"*. The plan ships `RunnerKind::AnthropicApi` (already the forward-pointer enum), `AnthropicApiRunner`, and an Invocation that is one Messages API request. No new term is coined; no existing term is overloaded. The `SendFn` seam name mirrors the existing `SpawnFn` — one vocabulary across the two runner impls.

## Refactor-before-add check

The plan adds a new module rather than refactoring an existing one — correct here. `claude_cli.rs` and `anthropic_api.rs` are two concrete impls of one trait; there is no shared body to factor out beyond `parse_verdict`/`parse_artifact`, which the plan **does** reuse (DD4 — already `pub` in `stream_json.rs`, no duplication). Collapsing the CLI runner's three-file split (`command`/`stream_json`/`claude_cli`) into one file for the API runner (DD-File-Structure) is justified by the smaller surface and does not strain the model. No addition a refactor would obviate.

## Findings

### F1 [low] §E-language-drift — "Invocation" is the API request, not the HTTP round-trip
**What:** The plan's module doc and the `SendFn` describe "the HTTPS POST" / "send the request". DOMAIN.md's unit of meaning is the **Invocation** (one Claude call). The seam returns a raw body string; readers could conflate the transport (`SendFn`) with the domain Invocation (`Runner::invoke`).
**Cited plan section:** Task 5 (`SendFn` doc-comment), File Structure (module doc).
**Why it matters:** Keeping the seam's vocabulary distinct from the domain term keeps the ACL legible — `invoke` is the Invocation; `SendFn` is the transport detail it seals. Pure language hygiene; no code-shape change.
**Suggested amendment:** Module doc states `invoke` = one Invocation and `SendFn` is the sealed transport (the HTTP round-trip), mirroring how `claude_cli.rs` frames `SpawnFn` as "the only Claude-idiom side effect (spawning the subprocess)".
**Status:** resolved — applied in the plan's `anthropic_api.rs` module doc + `SendFn` doc-comment (Task 5), which now name the Invocation/transport split explicitly.

### F2 [low] §E-boundary — name the convergent 429 paths so the seal reads as one rule
**What:** HTTP 429 → `RateLimited` is mapped in two places: the status check in `AnthropicApiRunner::new` (transport layer) and the `type:"error"` rate-limit envelope in `parse_response` (body layer). Both are inside the ACL (good — the seal holds), but a reader could read it as a duplicated rule rather than two layers converging on one `RunnerError::RateLimited`.
**Cited plan section:** Task 4 (`parse_response` error envelope), Task 5 (`new` status check), DD5.
**Why it matters:** The two-path 429 mapping is *correct* (a 429 may surface as either an HTTP status or a JSON error envelope depending on the gateway), but documenting it as "two layers, one outcome" prevents a future maintainer from "deduplicating" by deleting one path and losing a real case. This is the same discipline `stream_json.rs` uses for its rate-limit detection.
**Cited code:** `runners/src/stream_json.rs:33-46` (the CLI runner's single-layer rate-limit detection — the API runner's two-layer form is intentional, not an oversight).
**Suggested amendment:** A one-line comment at each 429 site noting the other path and that both converge on `RunnerError::RateLimited`.
**Status:** resolved — applied; both the `new` status branch and the `parse_response` envelope branch carry the convergence note in the plan (Task 4 / Task 5, DD5).

## Out-of-scope notes (not findings)

- **Live path not exercised** — the real HTTPS POST can't run headless (no key); request-build + response-parse + 429-mapping are fixture/fake-covered, the live call structural-only. This is a *test-coverage* caveat, correctly recorded in the plan and backlog, not a DDD design defect.
- **`invoke_stream` not overridden** — correct. The default trait method delegates to `invoke` (no deltas); R4's per-chunk live log is a CLI-runner feature, and the non-streaming Messages call has no prose-delta path. No "Log delta" / `LogSink` semantics are added or implied for this runner. Consistent with DOMAIN.md's Log delta being a streaming-worker concept.

## Verdict

**SOUND.** The design is a clean Runners ACL addition: a second concrete `Runner` behind the trait, the API idiom sealed inside `anthropic_api.rs` (no leak past `output.rs`), the runner-kind factory at the composition root with no new edge into Runtime, and naming that matches DOMAIN.md verbatim. Both findings are low-severity language/clarity items, both applied in-plan. Cleared to build.
