---
id: vet-r4-live-log-2026-06-23
verb: vet
target: plans/2026-06-23-plan-r4-live-log.md
lens: strategic · critique · brief
date: 2026-06-23
contexts: [Runners, Runtime, Conversational Control, LLM Chat]
---

# Vet — R4 Per-chunk live-log streaming from workers

**Scope of this gate:** DDD design soundness only (boundaries, ubiquitous language,
*Refactor before you add*). Decomposition/testability/sequencing are upstream
(writing-plans owns them). Read together: the plan, `DOMAIN.md`, and the affected
code (`runners/src/{output,stream_json,claude_cli,fake}.rs`, `runtime/src/pool.rs`,
`app/src/lib.rs`) plus the C2 parallels in `llm_chat`.

## Verdict

**Design is sound.** The change is genuinely additive and the seams hold: the only
thing crossing the `Runner` ACL for streaming is a `Box<dyn Fn(&str)>` prose
callback — no stream-json event type, JSON shape, or CLI flag leaks into Runtime.
The verdict/settle/route path and the two-aggregate Task / FanOutGroup model are
untouched (plan D5). The structure mirrors C2's already-merged chat-streaming
exactly, one ACL over.

Four findings, all minor/structural; none blocks the build. Apply F1–F3 (cheap,
recommended); F4 is a confirmed ruling recorded for the record.

---

## Findings

### F1 [minor] Off-language naming — register the live-log streaming vocabulary in DOMAIN.md

**What:** The plan introduces `invoke_stream`, `LogSink`, `LogSinkFactory`,
`task.log`, and `TaskLogBuffer` — a *streaming / log-delta* concept that the
Runners and Runtime ubiquitous-language sections of `DOMAIN.md` do not yet name.
Runtime's language today (`DOMAIN.md` → Runtime) tops out at **Settle** ("the
worker finishing; emits a verdict event") — a one-shot vocabulary. The live log is
a *new* business moment: the worker's output as it is produced, before settle.

**Cited plan section:** Tasks 1, 5, 6; Decisions D2, D6, D7. **Code:**
`runners/src/output.rs` (new `LogSink`), `runtime/src/pool.rs` (new `LogSinkFactory`).

**Why it matters:** *The language lives in the code.* New first-class types whose
concept isn't in `DOMAIN.md` is how a context's language quietly drifts from its
declared model. The AI Engineer already owns `stream-json` and `completion vs
streaming` in the roster — this is squarely their vocabulary; it just needs to be
written down so the next reader of `DOMAIN.md` finds it.

**Suggested amendment:** Add to `DOMAIN.md` → Runners (ACL): **Log delta** — a
display-only assistant-prose fragment the streaming worker invocation forwards as
it runs; carries no verdict/artifact and never influences settle. And to Runtime:
note that a worker invocation may **stream** log deltas (display-only) in addition
to its terminal **Settle** — streaming is a side channel, settle is the verdict
moment. One line each.

**Status:** resolved (DOMAIN.md updated 2026-06-23 — Runtime "Stream (live log)" + Runners "Log delta")

---

### F2 [minor] Unowned-shared-type guard — `LogSink` mirrors `DeltaSink`; keep them parallel, not merged

**What:** `runners::output::LogSink` is structurally identical to
`llm_chat::chat::DeltaSink` (`Box<dyn Fn(&str) + Send + Sync>`). The plan is right
to define a *separate* type in `runners` rather than importing `llm_chat`'s — but
the reason must be explicit so a future reader doesn't "DRY" them into one shared
module, which would couple two ACLs that are deliberately distinct
(`DOMAIN.md`: "Runners … stays the one-shot, verdict-shaped *worker* ACL"
vs LLM Chat the multi-turn chat ACL).

**Cited plan section:** Task 1 Step 3; Decision D1/D2. **Code:**
`llm_chat/src/chat.rs:75` (`DeltaSink`).

**Why it matters:** *One model per bounded context.* A shared `Sink` type would be
an unowned shared kernel spanning two ACLs — exactly the §E "unowned shared type"
smell, introduced by a well-meaning later refactor. The defence is a doc-comment
that names the parallel and forbids the merge.

**Suggested amendment:** The plan's Task 1 doc-comment already says "Mirrors
`llm_chat::chat::DeltaSink` (separate ACL crate, by design)" — keep that wording
verbatim and ensure it ships. No code change beyond the comment. (Already in the
plan; this finding just makes it a required, not optional, line.)

**Status:** resolved (required doc-comment specified in plan Task 1 Step 3)

---

### F3 [minor] Adds-where-a-refactor-fits / documented duplication — the incremental parser

**What:** `runners::stream_json::parse_stream_streaming` + the `feed`-returns-delta
change duplicate the shape of `llm_chat::stream_json::parse_chat_stream_streaming`.
The *Refactor before you add* law asks: could one parser serve both?

**Cited plan section:** Task 2; Decision D1. **Code:**
`llm_chat/src/stream_json.rs:130` (`parse_chat_stream_streaming`).

**Why it matters / ruling:** **No — keep the duplication, document it.** The two
parsers consume the same Claude wire idiom but produce *different domain outputs*:
`runners` parses verdict + artifact + usage into `RunnerOutput` (lenient
`VERDICT:`/`ARTIFACT:` convention, the AI-Engineer-vs-Engineer tension resolved
inside this ACL); `llm_chat` parses plain prose + a sealed `session_id` into
`ChatReply`. A shared parser would have to import both output idioms and the
session concept, leaking each ACL's vocabulary into the other — a worse coupling
than the duplication it removes. This is the *same* call C2 made and recorded; R4
is the symmetric case. This is therefore **documented duplication**, not a missed
refactor.

**Suggested amendment:** The plan's Task 2 doc-comment for `parse_stream_streaming`
already cross-references the `llm_chat` sibling — keep it, and make the same
"mirrors, by design" note on the `feed` return-type change so both halves of the
duplication are flagged. (Already in the plan; recorded here as the confirmed
ruling.)

**Status:** resolved (documented-duplication doc-comments required in plan Task 2)

---

### F4 [info] Event naming — `task.log {task_id, delta}` is the right shape; no `reset`

**What:** The plan names the Tauri event `task.log` with payload
`{task_id, delta}`, parallel to C2's `conversation.delta {text, reset}` (Decision
D6). It deliberately drops `reset` and adds `task_id`.

**Cited plan section:** Decision D6; Task 6 Step 3; Task 7.

**Why it's right (not a finding to fix):** The terminal is a *single* conversation,
so its delta stream needs a `reset` boundary between model steps. Worker logs are
*many concurrent streams* — the discriminator the frontend needs is *which task*,
not a step boundary; each claim is one invocation = one continuous stream. So
`task_id` replaces `reset` by the nature of the aggregate (many Tasks vs one
Conversation). The naming stays in the `<noun>.<event>` family already established
(`task.changed`, `conversation.delta`, `usage.changed`). Sound.

**One forward note (not blocking):** D6 acknowledges a reattempt does not clear the
prior run's buffer (the live-log tab keeps accumulating). That's an acceptable v1.1
display choice; if operators later want a per-attempt reset, that is a future
`task.log` `{reset}` addition — out of scope here. No amendment.

**Status:** resolved (design confirmed)

---

## Summary

| Finding | Severity | Smell (§E) | Disposition |
|---|---|---|---|
| F1 | minor | off-language naming | amend `DOMAIN.md` (register Log delta / streaming) |
| F2 | minor | unowned shared type | keep parallel types + the "by design" doc-comment |
| F3 | minor | adds-where-a-refactor-fits | documented duplication — confirmed, keep doc-comments |
| F4 | info | event naming | confirmed sound — no change |

No finding requires a pre-build refactor of existing code. F2 and F3 are already
satisfied by doc-comments the plan specifies; this gate makes them required. F1 is
a `DOMAIN.md` edit to apply alongside the implementation. Proceed to build.
