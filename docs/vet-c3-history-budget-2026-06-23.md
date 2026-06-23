---
id: vet-c3-history-budget-2026-06-23
verb: vet
target: plans/2026-06-23-plan-c3-history-budget.md
lens: strategic · critique · brief
date: 2026-06-23
reviewer: ddd-council (brief register — autonomous run)
verdict: SOUND — clean on §E; one advisory, no blockers
---

# DDD Vet — C3 accurate history-budget accounting for tool-call turns

**Target.** `plans/2026-06-23-plan-c3-history-budget.md` — rewrite
`Turn::estimated_tokens` (`conversational_control/src/turn.rs`) to fold the
serialized size of each embedded tool-call's `request.args` and `result` into the
per-turn token estimate, via a new private `tool_call_tokens` helper.

**Input of record.** Plan doc; `DOMAIN.md` (Conversational Control context, the
**History budget** ubiquitous-language term, the two declared shared kernels);
affected code `conversational_control/src/turn.rs`, `conversational_control/src/conversation.rs`,
`agent_bus_core/src/tool_protocol.rs`. This finding is the resolution of
composite-engine vet **F3** (which deferred exactly this estimate tweak to a
`turn.rs` follow-up).

## Scope confirmation

- **Kernel-only / acyclic — CONFIRMED.** The change lives entirely inside the
  `conversational_control` crate. It reads `agent_bus_core::{ToolCallRequest, ToolCallResult}`
  — already imported at `turn.rs:5` — which is the architecture-owned shared kernel
  (`DOMAIN.md` §Shared kernels item 2: "the OHS tool protocol `ToolSpec`,
  `ToolCallRequest`, `ToolCallResult`"). Reading a published kernel type is the
  sanctioned dependency direction; **no new edge enters any supplier context**, no
  cycle is introduced. The estimate stays where the language says it lives — the
  `Turn` value type inside the Conversation aggregate.
- **History-budget invariant correctly strengthened — CONFIRMED.** `DOMAIN.md`
  defines **History budget** as "token cap on the conversation; oldest exchanges
  drop out." The invariant is enforced in `conversation.rs::truncate_to_budget`
  (`conversation.rs:69`), which sums `Turn::estimated_tokens` (`conversation.rs:62-64`).
  Folding args + result into the per-turn estimate makes the sum a tighter
  lower-bound on real context size, so truncation fires when it should rather than
  late — the invariant is strengthened in the safe (conservative) direction. No
  change to `conversation.rs` is needed or proposed; it benefits transparently. The
  plan's choice to keep the `+ 8` structural constant and use byte `len()` (an upper
  bound on char count) both bias toward over-counting — correct for a budget cap.

## Findings (§E design-stage smells)

- **Cross-boundary dependency by design** — CLEAN. No reach past another context's
  public surface; only the kernel is read, and only types already in scope.
- **Unowned shared type** — CLEAN. No new shared/common type. `tool_call_tokens` is
  private to `turn.rs`; nothing new crosses a context line.
- **Off-language naming** — CLEAN. `tool_call_tokens` reads in the ubiquitous
  language (**Tool-call**, **History budget** / token estimate). No
  `Manager`/`Helper`/`Processor`/`Data` placeholder; the existing `estimated_tokens`
  name is preserved.
- **Adds where a refactor fits** — CLEAN (and correctly resolved). The plan reshapes
  the one place the estimate already lives rather than adding a parallel estimator.
  Extracting a private helper (D3) is a DRY refactor of the existing `.map(...).sum()`,
  not a new abstraction — the *Refactor before you add* law is honored.
- **Contradicts `DOMAIN.md`/spec** — CLEAN. The change *removes* a contradiction
  (composite-engine F3: the estimate under-counted the declared History-budget
  concept). It aligns code with `DOMAIN.md`, settles no concept, renames nothing.
- **Splits what changes together / couples what shouldn't** — CLEAN. Single-file,
  single-aggregate change; the args/result accounting changes in lockstep with the
  estimate it serves. No new seam, no synchronous chain.

### F1 [info] advisory — `+ 8` structural constant is now partly redundant but harmless

```
cited plan section: Decisions D1; Task 1 Step 3 (tool_call_tokens helper)
affected code:      turn.rs::Turn::estimated_tokens (old "+ 8" per call)
```

**What.** The plan keeps the legacy `+ 8` per-call constant *and* now charges the
serialized args (which include the JSON envelope `{...}` the constant previously
stood in for). There is mild double-counting of envelope overhead.

**Why it matters.** Negligible, and in the safe direction. For a budget cap, a small
over-estimate is strictly preferable to an under-estimate: it makes truncation
slightly more eager, never less. Removing the constant would risk a fat-turn-only
regression toward under-counting for tiny-arg calls. The plan's stated rationale
(D1: "the estimate only ever grows relative to v1 — never shrinks") is sound.

**Suggested amendment.** None required. Keep `+ 8` as written. Recorded only so the
double-count is a documented choice, not an accident.

**Status:** resolved — advisory accepted; no plan change. The constant is retained
deliberately per D1.

## Verdict

**SOUND.** The plan is clean on all six §E smells. It stays kernel-only and acyclic,
keeps the concept in the context that owns it, honors *Refactor before you add*, and
strengthens the History-budget invariant in the conservative direction. One info-level
advisory (F1), resolved with no change required. **Cleared to build.**
