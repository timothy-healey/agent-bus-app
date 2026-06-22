---
id: 2026-06-22-agent-bus-app-spec-vet
target: 2026-06-22-agent-bus-app-design.md
date: 2026-06-22
mode: vet
lens: strategic · design · workshop
operator: tim.healey@splose.com
---

# Vet — Agent Bus App spec

Pre-build review of `2026-06-22-agent-bus-app-design.md` against `DOMAIN.md` for DDD design soundness. Six findings, none blocking, all amendments to make before the implementation plan locks the spec. Severities low to medium.

## Acknowledged strengths

The spec gets the strategic shape right:

- The seven contexts and their relationship patterns (SK / CS / ACL / OHS) are correctly applied throughout. No leaky-boundary-by-design.
- The ACL through Runners is sealed: no Claude idiom (`--max-thinking-tokens`, `stream-json`, `cache_creation_input_tokens`) bleeds into Runtime's vocabulary.
- The hot-reload + `schema_version` pattern for the Pipeline kernel is correctly named as a *shared kernel with explicit versioning*, not an accidental kernel.
- The two-aggregate split in Runtime (Task vs WorkerPool, joined by reference) avoids the god-aggregate trap.
- Conversational Control's classification as a *customer* (not a fellow supplier) keeps tool-call surfaces explicit per-supplier.

The findings below are quality-of-language and unowned-types — not structural.

---

## Findings

### F1 [medium] cross-boundary-dependency-by-design — Tauri command organisation not specified

```
signalId:       cross-boundary-dependency-by-design
severity:       medium
cited section:  Architecture → Process model
                  "Tauri command surface (~30 RPC handlers)"
                  "The renderer (React) is a view layer over Tauri commands; never
                   does business logic, never spawns subprocesses, never touches
                   user files directly."
message:        The spec mentions ~30 Tauri command handlers but does not specify
                where they live. An implementer reading this could put all 30 in
                a single `src-tauri/src/commands.rs`, collapsing seven context
                boundaries into one file. That would be a cross-context-coupling
                smell created by omission.
suggestedAmendment:
                Add a paragraph to "Architecture → Process model" specifying:
                each context's `api` submodule defines its own Tauri commands;
                `src-tauri/src/lib.rs` only registers them. Equivalent to the
                DOMAIN.md note ("Each context exposing an `api` sub-module as its
                OHS") but said explicitly for Tauri command implementation.
```

**Why it matters.** Tauri commands ARE the contexts' public surface to the frontend. If the implementation puts them in one file, the boundary is dead at the language layer — every TS-side import pulls from `commands` regardless of which context it actually wants. The DDD model becomes paper.

**Status:** resolved (amendment applied) — small spec edit; no design rework needed.

---

### F2 [medium] off-language-naming — implementation components drift from DOMAIN.md vocabulary

```
signalId:       off-language-naming
severity:       medium
cited sections: Architecture → Process model
                  "Worker Manager (tokio task per worker)"
                  "Pipeline Engine (routes outboxes, manages cycles)"
                  "Usage Tracker (transcript watcher + SDK tail)"
                  "Scope Enforcer (per-invocation settings.json generation)"
                Various references to "god terminal" throughout
message:        Spec uses informal names that don't match DOMAIN.md's
                canonical vocabulary. Implementers will use the spec's names
                in code; reviewers will reference DOMAIN.md; the language
                strains.

                DOMAIN.md says                  Spec says
                ─────────────────────────       ─────────────────────────
                WorkerPool (aggregate)          Worker Manager
                Pipeline (aggregate) +          Pipeline Engine
                  routing inside Runtime
                Scope (per-invocation cfg)      Scope Enforcer
                Usage Telemetry (context)       Usage Tracker
                Conversation (aggregate)        god terminal
                  / Conversational Control
                  (context)
suggestedAmendment:
                Rename the spec's implementation components to align with
                DOMAIN.md aggregate/context names. Concretely:
                  "Worker Manager"  → "WorkerPool service" or just "WorkerPool"
                  "Pipeline Engine" → "Pipeline runtime" or "Pipeline router"
                  "Usage Tracker"   → "Usage Telemetry watcher" (the context)
                  "Scope Enforcer"  → "Scope policy" or "Scope settings writer"
                  "god terminal"    → "Conversation" (code) /
                                      "Terminal" (UI component)
                                      (keep "god terminal" only as casual UX
                                       reference in narrative text, never in
                                       code identifiers)
```

**Why it matters.** *The language lives in the code* is one of the council's shared laws. Drifting names create the §C "one concept, two names" smell: `WorkerManager` in Rust vs `WorkerPool` in DOMAIN.md vs "Worker Manager (tokio task per worker)" in the spec. Readers can't trust names; future critique runs flag this as a finding to fix.

**Status:** resolved (amendment applied) — text-only edit to the spec.

---

### F3 [medium] unowned-shared-type — Conversation aggregate has no persistence story

```
signalId:       unowned-shared-type
severity:       medium
cited sections: Data model → SQLite schema
                  (no `conversations` table)
                UI surfaces → God terminal
                  "Always-present at the bottom"
                DOMAIN.md → Conversation aggregate
                  "Sessions older than 24h get summarised on next launch"
message:        DOMAIN.md and the context-map specify a Conversation aggregate
                with an invariant requiring persistence across app restarts
                (the 24h-summarisation rule and session continuity). The spec's
                SQLite schema defines tasks, workers, comments, projects, two
                usage logs — but no `conversations` table. No file-based
                persistence is specified either. The aggregate has an enforceable
                invariant with nowhere to enforce it.
suggestedAmendment:
                Add to the SQLite schema:
                  CREATE TABLE conversations (
                    id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL,
                    started_at INTEGER NOT NULL,
                    last_message_at INTEGER NOT NULL,
                    history_json TEXT NOT NULL,        -- serialised turns
                    summary_of_prior_sessions TEXT,    -- when truncated/aged
                    FOREIGN KEY(project_id) REFERENCES projects(id)
                  );
                Or specify a file-based path:
                  .agent-bus/conversations/<session-id>.jsonl
                Recommend SQLite for v1 — keeps everything in one file,
                transactional with the rest of state. JSONL fine for v2 if
                conversation size becomes an issue.
```

**Why it matters.** Aggregate invariants without persistence are wishes. The 24h-summarisation rule and the UI's "always-present" claim both need a place to live across restarts. Adding the table is small; missing it after build means rewriting the Conversation layer.

**Status:** resolved (amendment applied).

---

### F4 [low] unowned-shared-type — Invocation aggregate's persistence/crash policy is implicit

```
signalId:       unowned-shared-type
severity:       low
cited sections: Worker model → Per-worker lifecycle
                  (no mention of Invocation persistence)
                DOMAIN.md → Invocation aggregate
                  "Once status == started, the request shape is locked"
message:        The Invocation aggregate has invariants (immutable once started,
                must record usage, must emit RateLimited on hit) but the spec
                doesn't say if Invocations are persisted at start or kept in
                process memory only. On a Rust process crash mid-invocation,
                the Task is released (via claim recovery) but any partial
                response stream the Invocation captured is lost.

                For v1 this may be acceptable (ephemeral Invocations; released
                task re-enters inbox; whatever Claude was doing is forgotten).
                But it should be a documented decision rather than an
                implementation accident.
suggestedAmendment:
                Add to "Worker model → Per-worker lifecycle":
                  "Invocations are kept in process memory only (v1 design).
                   On Runtime crash mid-invocation, the partial response is
                   lost; the Task's claim is released on restart and re-enters
                   the inbox to be retried by a fresh Invocation. Aggregate
                   invariants apply only while the process is alive.
                   v1.1 may persist Invocation start/completion for audit."
```

**Why it matters.** Without the explicit note, future implementers (or critique runs) will see invariants without persistence and flag it as a smell. With the note, it's a deliberate v1 design boundary.

**Status:** resolved (amendment applied).

---

### F5 [low] contradicts-domain.md-spec — YAML example uses v1.1 runner kind in a v1 section

```
signalId:       contradicts-domain.md-spec
severity:       low
cited sections: Data model → Pipeline definition
                  "  - id: spec-writers
                       runner:
                         kind: anthropic-api    ← v1.1 only"
                v1 boundaries — what ships first
                  "Worker manager + claude-cli runner (API runner v1.1)"
message:        The YAML example shows both runner kinds (claude-cli for
                research, anthropic-api for spec-writers) to illustrate the
                per-team-choice feature. But the v1 boundaries section says
                anthropic-api ships in v1.1. An implementer skimming the
                example concludes API runner is v1; reading the boundary
                section concludes it isn't. The spec contradicts itself
                across two sections.
suggestedAmendment:
                Add a one-line note to the YAML example:
                  "# Note: anthropic-api runner ships in v1.1; v1 supports
                  #       only claude-cli. Example shows both for illustration."
                Or replace the example's anthropic-api with claude-cli so v1
                examples reflect v1 behaviour. Recommend the inline note —
                keeps the example useful as a forward-pointer.
```

**Why it matters.** Spec contradictions surface as implementation rework. The example is read more carefully than the boundaries section by many implementers; aligning them prevents wasted work.

**Status:** resolved (amendment applied).

---

### F6 [low] unowned-shared-type — tool catalog ownership across contexts unspecified

```
signalId:       unowned-shared-type
severity:       low
cited sections: UI surfaces → God terminal
                  "Claude has app-specific tools matching the canonical actions"
                  (table of tools follows; no ownership/registration story)
                Chunk 1 brainstorm
                  "Each supplier publishes its app-tool surface as Open-Host
                   Service"
message:        The terminal's available tool catalog is the union of every
                supplier's published app-tools (inject_topic, approve_gate,
                brake_on, etc.). The spec doesn't say where the union catalog
                lives. Three possible failures:
                  - A god module `src/tools.rs` lists every tool from every
                    supplier (unowned shared type)
                  - The catalog is persisted in SQLite and drifts from code
                    (stale catalog vs current implementation)
                  - Each supplier registers its tools at runtime through a
                    global mutable registry (shared mutable state across
                    contexts; testing nightmare)

                None are explicit in the spec.
suggestedAmendment:
                Add to "UI surfaces → God terminal" or "Architecture":
                  "Tool catalog is constructed at startup by querying each
                   supplier's `api::tools() -> Vec<ToolSpec>` function. The
                   catalog is in-memory, read-only after construction, and
                   never persisted. Adding a new app-tool requires touching
                   exactly two places: the supplier's `tools()` function and
                   the terminal's tool-call UI affordance — by design (per
                   chunk 1 'adding a new app-tool is a cross-context change')."
```

**Why it matters.** "Where does this thing live" is the most common implementation question. Without the spec naming it, the first implementer to touch tools improvises; the second improvises differently; the catalog ends up as one of the three failures above. Specifying now is one paragraph; fixing later is a refactor.

**Status:** resolved (amendment applied).

---

## Roll-up

Six findings, none blocking. Five are short text edits; F3 (Conversation persistence) is the only one requiring a schema addition. All within scope of pre-implementation spec polish.

The DDD bones of the spec are sound. Apply these amendments before invoking the implementation-plan skill so the plan locks against a clean target.

**Recommended order of fixes:**
1. F2 — rename components (touches many sections; do first so later edits use the new names)
2. F1, F6 — add Tauri command organisation paragraph + tool catalog paragraph
3. F3 — add `conversations` table to SQLite schema
4. F4 — add Invocation crash-policy note
5. F5 — add inline note to YAML example

---

## Resolution — 2026-06-22

All six amendments applied to `2026-06-22-agent-bus-app-design.md` in one pass. Summary of changes:

- **F2** — renamed component references in spec: `Worker Manager` → `WorkerPool service`; `Pipeline Engine` → `Pipeline router`; `Usage Tracker` → `Usage Telemetry watcher`; `Scope Enforcer` → `Scope policy`. "god terminal" replaced with "Conversation (god-terminal UI)" in implementation contexts; kept as casual UX reference in narrative text where appropriate.
- **F1** — added "Tauri command organisation" subsection to "Architecture" with per-context `api::register(builder)` pattern and concrete `lib.rs` aggregation example.
- **F6** — added "Tool catalog ownership" paragraph to "UI surfaces → God terminal" specifying build-time aggregation from each supplier's `api::tools()` function; read-only, in-memory, never persisted.
- **F3** — added `conversations` table to SQLite schema with `id`, `project_id`, `started_at`, `last_message_at`, `history_json`, `summary_of_prior_sessions` columns + index.
- **F4** — added "Invocation crash policy (v1)" paragraph to "Worker model" specifying ephemeral Invocations + Task-level claim recovery for v1; persistence deferred to v1.1.
- **F5** — added inline comment to spec-writers YAML example noting the `anthropic-api` runner is a forward-pointer to v1.1; v1 ships only `claude-cli`.

Pass 1 amendments applied.

---

## Vet pass 2 — 2026-06-22

Re-vetting after pass 1. F1–F6 confirmed clean (no stale "Worker Manager" / "Pipeline Engine" / "Usage Tracker" / "Scope Enforcer" remaining; `conversations` table present; god terminal references only appear in narrative copy). Two new findings raised by the pass-1 amendments themselves.

### F7 [medium] unowned-shared-type — no shared kernel defined for cross-context primitive types

```
signalId:       unowned-shared-type
severity:       medium
cited sections: Architecture → Tauri command organisation
                  "pub async fn inject_topic(...) -> Result<TaskId, RuntimeError>"
                UI surfaces → God terminal → Tool catalog ownership
                  "pub fn tools() -> Vec<ToolSpec>"
                DOMAIN.md → Notes for the detector
                  (Workspace kernel = paths; no kernel for cross-context types)
message:        Pass-1 amendments introduced code examples referencing types
                used across context boundaries: `TaskId`, `ToolSpec`,
                `ToolCallRequest`, `ToolCallResult`, `Verdict`,
                `RunnerKind`, `EffortMode`, `ProjectId`, `TeamId`,
                `PipelineId`, `ArtifactPath`. The spec doesn't say where
                these primitives live.

                Without a defined home, implementers will improvise: some
                will re-export from each context, some will define in
                whichever context first needs them, some will create
                ad-hoc `types.rs` files inside arbitrary crates. Result:
                the same identity (e.g. TaskId) exists in three places,
                or one context becomes an undeclared dependency of
                another.

                This is a second shared kernel — small, justified, named.
                Distinct from Workspace's path kernel (data-shaped); this
                one is type-shaped.
suggestedAmendment:
                Add a section to "Architecture" or "Data model" titled
                "Cross-context primitives". Specify:
                  - A small Rust crate (e.g. `agent_bus_core`) holds:
                    ID types (TaskId, TeamId, PipelineId, ProjectId,
                    ArtifactPath); OHS protocol types (ToolSpec,
                    ToolCallRequest, ToolCallResult); cross-context
                    enums (Verdict, RunnerKind, EffortMode).
                  - Every context crate depends on `agent_bus_core`.
                  - `agent_bus_core` depends on nothing in the project
                    (only stdlib + serde + tauri-types). This makes it
                    a true kernel — no cycle risk.
                  - Update DOMAIN.md "Notes for the detector" to list
                    `agent_bus_core` as a second shared kernel alongside
                    Workspace's path kernel.
                  - Update `ddd-council.json` config (when written) to
                    include `agent_bus_core` as a kernel module so the
                    detector doesn't flag every other context importing
                    from it.
```

**Why it matters.** Cross-context primitives without a defined kernel are the canonical accidental-shared-kernel smell, by omission. Specifying now is one paragraph; fixing later requires identifying every duplicate, picking the canonical version, updating every import. Cheap upfront, expensive after.

**Status:** open — needs a paragraph added to the spec + a note in DOMAIN.md.

---

### F8 [low] contradicts-domain.md-spec — Open Questions section contains stale resolved items

```
signalId:       contradicts-domain.md-spec
severity:       low
cited sections: Open questions
                  "Should the god terminal persist its conversation
                   across app restarts? ... Lean: keep last 24h, summarise older."
                  "Worktree base directory — under the project root
                   (current proposal) or under a separate ~/agent-bus-worktrees/?"
                  "Skill / plugin discovery for CLI runner..."
message:        Three of the four Open Questions were resolved during the
                design session, but the spec section still lists them as
                open. Implementers reading the section will think these
                are unsettled and could re-litigate or improvise.

                - "god terminal persist" → resolved (yes; `conversations`
                  table added in F3)
                - "Worktree base directory" → resolved (under project
                  root, per the Git integration section's path template)
                - "Skill / plugin discovery for CLI runner" → resolved
                  (trust the team's prompt to reference plugins by name;
                  no runtime discovery needed in v1)

                Only the Berkeley Mono cost question remains genuinely
                open.
suggestedAmendment:
                Rewrite "Open questions" to:
                  1. Remove the three resolved items, OR move them to a
                     "Resolved during design" subsection for the audit
                     trail.
                  2. Keep Berkeley Mono cost as the lone open question.
                  3. Optionally add: "All other questions raised during
                     brainstorm/vet were resolved; see commit history
                     and `docs/vet-...md`."
```

**Why it matters.** A spec is read more carefully than its commit history. Stale Open Questions undermine reader confidence ("are these really decided, or did the author forget?"). Cheap to clean up.

**Status:** open — single section rewrite.

---

### Resolution gate

Pass-1 findings (F1–F6) are all resolved. Pass-2 raised F7 (medium) and F8 (low).

---

## Resolution — pass 2 — 2026-06-22

F7 and F8 amendments applied to `2026-06-22-agent-bus-app-design.md` and `DOMAIN.md`.

- **F7** — added "Cross-context primitives (shared kernel)" subsection under Architecture in the spec. Defines the `agent_bus_core` Rust crate, lists its modules (`ids`, `verdict`, `runner`, `tool_protocol`) and the types in each, plus hard rules (depends on nothing project-internal, no business logic, no re-exports from contexts). Updated DOMAIN.md "Notes for the detector" to add a `kernels` block listing both `agent_bus_core` and Workspace's path kernel; added a "Shared kernels" section above the detector notes documenting both as deliberate, owned, named contracts.

  **Status:** resolved (amendment applied)

- **F8** — rewrote Open Questions section. Berkeley Mono cost remains as the lone open question (genuinely undecided). Three resolved items moved to "Resolved during design (kept for audit trail)" with brief pointers to where each was settled (Git integration section · Data model → SQLite schema · the brainstorm decision on plugin discovery).

  **Status:** resolved (amendment applied)

### Final state

All 8 findings across two vet passes are resolved. The spec is `vet`-clean. Ready for `superpowers:writing-plans`.
