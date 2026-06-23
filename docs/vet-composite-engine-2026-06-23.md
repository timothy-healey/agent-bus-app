---
id: 2026-06-23-composite-engine-vet
target: plans/2026-06-23-plan-composite-engine.md
date: 2026-06-23
mode: vet
lens: strategic · design · workshop
operator: tim.healey@splose.com
---

# Vet — composite engine plan (backlog C1, within-turn agentic-loop variant)

Pre-build DDD review of the `CompositeEngine` + `AgenticChatEngine` plan against
`DOMAIN.md`, the `Conversation`/`Turn`/`ToolCall` aggregate, the engine + dispatch
seams, the `Brake`, and the `llm_chat` ACL. Four findings — two low, two
operator-gated forks (low/medium) — plus three explicit confirmations the plan
asked the council to make. None blocks the build.

## Acknowledged sound

- **One composite Turn = the correct reading of the Turn invariant (DD2/D5).**
  DOMAIN.md defines a **Turn** as "one user message + one assistant response (with
  embedded tool-calls)". The loop accumulates every dispatched `ToolCall` into one
  `EngineReply.tool_calls` and lets `send_message_inner` record exactly one user
  Turn + one assistant Turn. Alternation (`conversation.rs` `append`) holds; the
  N-turns alternative would trip `ConversationError::NonAlternating`. `api.rs` /
  `conversation.rs` / `turn.rs` are correctly left untouched.
- **`ToolCall` already records the result.** `turn.rs` defines
  `ToolCall { request, result: Option<ToolCallResult> }`; the loop sets
  `result: Some(result)`. No aggregate change is needed to persist what each tool
  returned (see confirmation (b)).
- **Customer purity + dependency direction (D1).** The loop reads the `Brake`
  (Runtime) and dispatches through the **existing** `ToolDispatcher` trait — it
  adds no new edge into `conversational_control`, which stays a kernel-only
  customer. The root (`app`) is the one place allowed to import every context;
  composing the brake there is the designed seam, not a leak (see confirmation (c)).
- **Loop safety is treated as a real invariant (D6/DD1/DD4).** `MAX_STEPS` bound,
  brake-before-step, brake-before-dispatch, rate-limit/error stop, and
  invalid-call-fed-back-never-panic are each pinned by a named test (Tasks 5/4/2).
- **Slash routing reuses the parser's own rule (D2).** Routing by
  `trim_start().starts_with('/')` is the single source of truth for "what is a
  command", so a malformed `/cmd` surfaces as a visible error turn rather than
  silently entering the loop.

## Findings

### F1 [low] adds-where-a-refactor-fits — `extract_tool_call_block` re-implements the wizard's `extract_json_block`

```
cited plan section: DD3; Task 1 (Step 3); Orientation ("the fenced-JSON extraction to mirror")
affected code:      pipeline/src/design_session.rs::extract_json_block (lines 18–39)
                    app/src/lib.rs (new extract_tool_call_block)
```

**What.** DD3 + Task 1 add a local `extract_tool_call_block` that, by the plan's
own words, "mirrors `pipeline::design_session::extract_json_block`" — the same
prefer-```json-fence, fall-back-to-bare-fence logic, copied. The stated reason is
"so the agentic loop does not couple to the wizard module."

**Why it matters.** This is the §E *Adds where a refactor fits* law: two identical
fenced-JSON extractors will drift (one gets a bug fix the other misses), and the
"avoid coupling to the wizard" rationale is thin — the model's fenced-emit
convention is a cross-cutting concern both the wizard and the terminal share, and
both sit at/near the composition layer. A shared extractor would carry the model
once.

**Suggested amendment (operator fork).** Either (a) lift the extractor into a tiny
shared home both callers reach (e.g. a free function in `agent_bus_core` or a
small root-level util) and have both `pipeline` and `app` call it — paying the
refactor now; or (b) accept the deliberate duplication and say so explicitly,
noting the two copies must be kept in sync (a one-line comment cross-referencing
`design_session::extract_json_block`). The plan currently picks (b) implicitly;
the council recommends making that choice explicit, and prefers (a) if the
extraction logic is expected to evolve (native tool-use, per the roadmap).

**Status:** RESOLVED (2026-06-23, implementation) — operator chose (b) explicit
documented duplication. `extract_tool_call_block` is kept local to
`app/src/lib.rs` with a doc-comment cross-referencing
`pipeline::design_session::extract_json_block` ("keep the two in sync"). Low
blast radius; not a build blocker.

### F2 [low] off-language-naming — `AgenticChatEngine` / `CompositeEngine` / `ParsedToolCall` are not in the ubiquitous language

```
cited plan section: D4; D7; Task 2 (ParsedToolCall, AgenticChatEngine);
                    Task 6 (CompositeEngine); File-structure diff
affected code:      DOMAIN.md → Conversational Control language
                    (Conversation, Turn, App-tool, Tool-call)
```

**What.** The plan introduces `AgenticChatEngine`, `CompositeEngine`,
`ParsedToolCall`, `build_agentic_system_prompt`, and `tool_names`. None appears in
DOMAIN.md's Conversational Control vocabulary (Conversation, Turn, App-tool,
Tool-call). "Composite" and "Agentic" are implementation adjectives, not domain
words; `ParsedToolCall` is a near-synonym for the kernel's `ToolCallRequest`.

**Why it matters.** §E *Off-language naming* — new names that don't match the
ubiquitous language put the wrong language in the code. **Mitigant:** these are
composition-root engine *strategies*, not domain aggregates, and the domain-facing
seam (`ConversationEngine`, `EngineReply`, `Turn`, `ToolCall`) is reused verbatim.
The introduced terms describe *how* the terminal is wired, a layer DOMAIN.md
doesn't name, so the bar is lower than for a domain type — but `ParsedToolCall`
specifically shadows a kernel concept and is worth a second look.

**Suggested amendment.** Keep `CompositeEngine`/`AgenticChatEngine` (they are
honest names for engine strategies at the root, and `ConversationEngine` already
establishes the `*Engine` suffix in this crate). Consider renaming
`ParsedToolCall` → something that reads as "the model's emitted tool-call before
validation" (e.g. `ToolCallEmit` or simply deserializing straight into a
`{ tool, args }` that maps to `ToolCallRequest`), to avoid a second name for the
kernel's tool-call concept. Optionally add a one-line note in D4/D7 stating these
are root-composition strategy types, not additions to the Conversational Control
ubiquitous language.

**Status:** RESOLVED (2026-06-23, implementation) — operator chose to DROP
`ParsedToolCall` entirely. The fenced `{ "tool", "args" }` block is parsed
directly into the kernel's `ToolCallRequest { tool_name, args }` via a local
`parse_tool_call` helper, so there is no second name for the kernel's tool-call
concept. The two `*Engine` names are kept (acknowledged sound). Not a build
blocker.

### F3 [low] contradicts-DOMAIN.md (history-budget) — the composite Turn's token estimate ignores args + results

```
cited plan section: DD2; D5; "DDD vet — RECOMMENDED before building" item 1
affected code:      conversation.rs::truncate_to_budget / estimated_tokens;
                    turn.rs::Turn::estimated_tokens (name.len()/4 + 8 per call)
```

**What.** A composite Turn now embeds N `ToolCall`s, each carrying a real
`ToolCallRequest.args` and a `ToolCallResult` (the result JSON the loop fed back).
`Turn::estimated_tokens` charges only `tool_name.len()/4 + 8` per call — it counts
neither the args nor the result payload. The agentic loop makes turns materially
fatter than v1's single-tool slash turns, so the **History budget** invariant
("total history tokens ≤ history_budget_tokens, drop oldest pair") will
systematically under-count and let the real context window grow past budget.

**Why it matters.** §E *Contradicts DOMAIN.md* — **History budget** is a declared
Conversational Control concept ("token cap on the conversation; oldest exchanges
drop out"). An estimate that is right for v1 but wrong for fat composite turns
quietly erodes the invariant the concept exists to protect. Blast radius is low
(an over-long prompt / a late truncation, not a crash), and the budget defaults to
8 000 tokens which absorbs small loops — but the gap is real and grows with loop
length.

**Suggested amendment.** Note in DD2 (or a new task) that
`Turn::estimated_tokens` should fold in `args` + `result` size for embedded
tool-calls once composite turns land — even a coarse `serde_json` length / 4 per
call is closer than the current constant. This is a `turn.rs` change (the one place
the estimate lives), so it touches `conversational_control` — flag it as a *future*
follow-up rather than smuggling it into this plan, which is correctly scoped to
leave the kernel untouched. The plan already half-raises this (vet-recommended
item 1); this finding makes it explicit and pins the file.

**Status:** open — recommend tracking as a follow-up to `turn.rs`; not in scope for
C1 and not a build blocker.

### F4 [low] leaked-invariant (latent) — brake-before-dispatch leaves a dispatch in flight uncancellable

```
cited plan section: DD4; D6; Task 5 (braked_loop_stops_before_any_model_call_or_dispatch)
affected code:      runtime/src/brake.rs (is_on); app RootDispatcher.dispatch
```

**What.** The loop checks `brake.is_on()` before each `chat` call and before each
`dispatch`, but a dispatch already in flight when the brake flips is not
cancelled. The **Brake** concept is "halt new claims; in-flight workers complete"
(DOMAIN.md / `brake.rs`), so a single in-flight tool-call completing after the
brake flips is consistent with the brake's contract — but the plan should say so
rather than leave it implicit, because the terminal's loop is a *new* brake
consumer and a reader might expect mid-step cancellation.

**Why it matters.** Low — it matches the brake's documented "in-flight completes"
semantics, so it is not a violation; the risk is only that the boundary is
unstated. The plan's roadmap already names "cooperative cancel mid-dispatch" as a
follow-up.

**Suggested amendment.** Add one sentence to DD4: "consistent with the Brake's
'in-flight completes' contract, a tool-call already dispatched when the brake flips
runs to completion; the brake gate prevents the *next* step/dispatch, not mid-call
cancellation." No code change.

**Status:** open — one-line clarification recommended; matches existing brake
semantics, not a build blocker.

## Confirmations the plan requested

**(a) Composite Turn preserves alternation; internal round-trips do not leak into
the aggregate. CONFIRMED.** `send_message_inner` (per Orientation, line 72) appends
exactly one user `Turn` then one assistant `Turn::assistant(reply.text,
reply.tool_calls, now)`. The loop's `next_user_message` framing strings exist only
as `ChatRequest.user_message` handed to `runner.chat` — they reach the **llm_chat
session** (same `dialogue_id`, F3-sealed) and are **never** written to the
`ConversationStore` as user Turns. Alternation holds (one user → one assistant);
no fake user Turns are smeared into the `Conversation` aggregate.

**(b) Does `ToolCall` need a result field? RULING: NO — it already has one.**
`turn.rs` defines `ToolCall { request: ToolCallRequest, result: Option<ToolCallResult> }`,
and the loop sets `result: Some(result)` for every dispatched call. The composite
Turn therefore persists exactly what each tool returned. No aggregate change. (One
caveat lands as F3: persisting fat results means the history-budget estimate should
eventually account for them — but that is an estimate tweak, not a missing field.)

**(c) `conversational_control` stays kernel-only / acyclic. CONFIRMED.** All new
code lives in `app/src/lib.rs`; the loop reaches suppliers only through the existing
`ToolDispatcher` trait and reads the `Brake` at the root. No new edge enters
`conversational_control`; no crate depends back on `app`. Task 9's checks
(`git diff --stat` empty; `cargo tree -i llm_chat`/`-i runtime` → no match;
`cargo tree -i conversational_control` on `llm_chat` → no match) are the right
guards and are present in the plan.

## Roll-up

No blocking findings. F1 (duplicate extractor) and F2 (`ParsedToolCall` naming)
are genuine operator-gated forks with a recommendation each; F3 (budget estimate)
and F4 (brake-in-flight) are low-severity clarifications best tracked as
follow-ups outside this plan's deliberately-narrow scope. The three confirmations
the plan asked for all come back green: the composite-Turn semantics are sound, the
result field already exists, and the kernel stays pure and acyclic.

**Verdict: C1 PLAN READY FOR IMPLEMENTATION** — pending two low/medium operator
rulings (F1, F2) that change wording/one type name, not structure.
