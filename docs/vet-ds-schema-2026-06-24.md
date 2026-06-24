---
id: vet-ds-schema-2026-06-24
verb: vet
mode: vet
lens: strategic · critique · brief
target: plans/2026-06-24-plan-ds-schema.md
domain_refs: [DOMAIN.md, docs/context-map.md]
date: 2026-06-24
verdict: SOUND WITH FIXES
---

# Vet — DS-Schema: schema-enforced design-session output

**Change under review.** Generate each Design Session step's JSON Schema from the
`Slice` Rust types via `schemars` and embed it in the system prompt (one source of
truth, no prompt↔parser drift); turn a malformed/invalid emission from a silent
no-op into a bounded model-driven repair loop (2 retries) in both `kickoff_generate`
and `design_session_turn`. `best_effort_validate` (semantic, non-blocking) unchanged.

**Scope of the gate (DDD soundness only).** Boundaries, language, *refactor-before-add*.
Decomposition / testability / sequencing are upstream (the plan is TDD, commit-per-task,
full code) and out of scope here.

## Footing

- **Context owning the change:** Pipeline Authoring (the Design Session, `pipeline/src/design_session.rs`; the slice types in `pipeline/src/draft.rs`; the node types in `pipeline/src/model.rs`). All three files are inside the one context — confirmed against `DOMAIN.md` → Bounded contexts.
- **ACL it consumes:** LLM Chat (ACL) via `ChatRequest`/`ChatRunner`. The plan adds no new method to the trait, no new field to `ChatRequest`/`ChatReply`, and seals no CLI idiom out of place.
- **Constraint of record (the operator's framing, confirmed sound):** the Design Session runs over the **claude-cli** path; `claude --print` cannot define custom tool `input_schema`s or force `tool_choice`. Native structured-output/forced-tool-use is an **anthropic-api** (R1 runner) capability, not on the CLI path. The plan's shape — derived-schema-in-prompt + structural parse + bounded repair — is the correct CLI-path hardening; native tool-use is correctly deferred to a backlog roadmap item, not attempted here. The room concurs (AI Engineer: this is exactly the prompt-fragility-vs-strict-validation tension, resolved the right way for a slippery CLI surface — soft-enforce in the prompt, hard-validate the parse, self-correct on miss).

## Findings

### F1 [low] Off-language naming — the "repair turn" concept is unregistered ubiquitous language

**What.** The plan introduces a genuinely new Design Session concept — a bounded
**repair turn**: on extract-or-parse failure the model is re-prompted on the same
`dialogue_id` with the specific error, up to 2 retries, before giving up. This is a
real domain moment (a named state transition in the authoring dialogue), but it is
not in `DOMAIN.md`'s Pipeline Authoring → Design Session entry, and the helper names
(`chat_with_repair`, `repair_user_message`, `MAX_REPAIR_RETRIES`) put a term in the
code that the ubiquitous language doesn't yet define.

**Cited plan section.** `## Decisions` D-REPAIR-COUNT / D-REPAIR-DIALOGUE / D-REPAIR-SURFACE; Task 6 (`chat_with_repair`, `repair_user_message`, `MAX_REPAIR_RETRIES`).

**Why it matters (→ §C / §E off-language naming).** *The language lives in the code.*
A new named concept should be registered so the term means one thing across the
context and future work doesn't reinvent it ("retry" vs "repair" vs "re-emit"). Low
severity — the names are good and consistent; the gap is documentation, not design.

**Amendment.** Register **repair turn** in `DOMAIN.md` under Pipeline Authoring →
Design Session: "a bounded re-prompt (≤2 retries) on the same `dialogue_id` when the
model's reply has no fenced ```json block or the block fails the slice schema; names
the specific failure and asks for a re-emit. Distinct from a semantic issue, which
`best_effort_validate` surfaces non-blocking and never triggers a repair." Keep the
`*_repair*` names as-is. **Apply in-plan** (add a doc step to Task 8 or fold into the
backlog DONE entry).

**Status:** resolved

### F2 [low] Off-language naming — "schema" is freshly load-bearing; name the single-source-of-truth rule

**What.** The plan makes the **derived slice schema** a first-class artifact of the
Design Session (embedded in every step prompt; the thing the repair loop re-points
the model at). `DOMAIN.md` mentions `schema_version` (the YAML format version) but
not this new sense of "schema" (a `schemars`-derived JSON Schema of a `Slice`). Two
different senses of one word inside one context is a §C smell unless disambiguated.

**Cited plan section.** D-SCHEMA-SRC, D-SCHEMA-FORM; Task 4 (`slice_schema`, `step_system_prompt`).

**Why it matters (→ §C the names lie).** Reader could conflate the pipeline
`schema_version` with the emit `slice schema`. The plan's prose already distinguishes
them, but the language registry should too, and should name the *invariant* (the
schema in the prompt is GENERATED from the slice type — never hand-written).

**Amendment.** In the same DOMAIN.md Design Session entry, add: "**Slice schema** —
the JSON Schema of a `Slice` payload, *derived* from the Rust slice type via
`schemars` and embedded in the step's system prompt (single source of truth: the
prompt and `parse_slice` can never drift). Distinct from `schema_version`, the
pipeline YAML format version." **Apply in-plan** alongside F1.

**Status:** resolved

### F3 [info] No boundary leak — repair message is composed in the consumer, correctly (confirm-only)

**What.** The repair re-prompt (`repair_user_message`) is built inside Pipeline
Authoring and passed as an ordinary `ChatRequest.user_message`. The room checked
whether composing a "re-emit per the schema" instruction leaks chat-protocol or
CLI-idiom knowledge across the LLM Chat ACL.

**Cited plan section.** Task 6 `repair_user_message` / `chat_with_repair`.

**Finding.** No leak. The Design Session already owns "what to say to the model" (it
composes all four system prompts and the `turn_user_message`); a repair user-message
is the same kind of content. Nothing CLI-shaped (`session_id`, `--resume`,
stream-json) crosses — continuity is the existing `dialogue_id`, which is the ACL's
published handle. The ACL stays untouched and kernel-only. This is the *right* place
for the repair policy: the retry budget is a Pipeline-Authoring authoring rule, not a
transport concern. **No amendment.**

**Status:** resolved (no action)

### F4 [info] Refactor-before-add holds; `JsonSchema` on Fork/Join/Gate is in-context (confirm-only)

**What.** The plan adds a `schemars::JsonSchema` derive to `Fork`/`Join`/`Gate`
(`model.rs`) because `WiringSlice` embeds them. Checked for *unowned shared type* /
*adds where a refactor fits*.

**Cited plan section.** Task 2; Task 4 D-SCHEMA-FORM.

**Finding.** Sound. `Fork`/`Join`/`Gate` are Pipeline Authoring types (Pipeline
aggregate node types) in the same crate/context that owns the slices — the derive is
not a new shared kernel and crosses no boundary. The change is *reshaping* the prompt
builders (replacing four hand-written `const` shape literals with derived-schema
`fn`s), not adding a parallel abstraction beside them — the prompt `const`s are
deleted, not duplicated. Correctly refactor-first. The plan's own note (if a derive
snags, derive on the referenced type / thin schema struct) is the right adaptation
and stays in-context. **No amendment.**

**Status:** resolved (no action)

## Verdict

**SOUND WITH FIXES.** The design keeps schema derivation and the repair loop wholly
inside Pipeline Authoring / the Design Session; the LLM Chat ACL is unaffected; the
derived schema is owned by the slice types (one source of truth, drift-proof); no
boundary leak. The two findings (F1, F2) are language-registration only — register
**repair turn** and **slice schema** in `DOMAIN.md` and the plan is clear to build.
Both applied in-plan. F3/F4 are confirm-only (no action).
