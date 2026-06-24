---
id: 2026-06-22-agent-bus-app-context-map
target: ~/assistant/Efforts/agent-bus-app/2026-06-22-agent-bus-app-design.md
date: 2026-06-22
mode: design (map)
lens: strategic · design · workshop
operator: tim.healey@splose.com
---

# Agent Bus App — Context Map

A strategic + tactical DDD model derived from the system design spec. Produced via `/ddd-council` `map` verb in workshop register, with all decisions ratified by the operator in-session. This document is the source of truth for context boundaries, relationships, aggregate roots, and invariants once code lands.

## Intended context map

```mermaid
flowchart TB
    subgraph customer[" "]
        CC[Conversational Control<br/><i>customer · 1 aggregate</i>]
    end

    subgraph kernel[" "]
        WS[Workspace<br/><i>shared kernel for paths</i>]
    end

    PA[Pipeline Authoring<br/><i>shared kernel for graph</i>]
    RT[Runtime<br/><i>2 aggregates</i>]
    RV[Review<br/><i>1 aggregate</i>]
    RN[Runners ACL<br/><i>1 aggregate</i>]
    UT[Usage Telemetry<br/><i>1 aggregate</i>]
    CCT[(Claude Code transcripts<br/><i>external</i>)]
    CL[(Claude API / CLI<br/><i>external</i>)]

    WS -. SK .-> PA
    WS -. SK .-> RT
    WS -. SK .-> RV
    WS -. SK .-> UT
    WS -. SK .-> RN
    WS -. SK .-> CC

    PA <-. SK · hot-reload .-> RT

    RT -- CS · "task at gate" --> RV
    RV -- conformist · verdict --> RT

    RT -- ACL · "invoke(task, scope)" --> RN
    RN -- "translate to/from" --> CL

    RN -- CS · usage events --> UT
    CCT -- conformist · JSONL --> UT

    CC -- "customer of OHS" --> PA
    CC -- "customer of OHS" --> RT
    CC -- "customer of OHS" --> RV
    CC -- "customer of OHS" --> UT
    CC -- "customer of OHS" --> RN
    CC -- "customer of OHS" --> WS

    classDef supplier fill:#0a1a2e,stroke:#1f6feb,color:#e6edf3;
    classDef customer fill:#2a1a45,stroke:#a371f7,color:#e6edf3;
    classDef kernel fill:#3d2e0a,stroke:#d29922,color:#e6edf3;
    classDef external fill:#1a1a1a,stroke:#888,color:#aaa,stroke-dasharray:4 4;
    class PA,RT,RV,RN,UT supplier;
    class CC customer;
    class WS kernel;
    class CCT,CL external;
```

**Legend.** `SK` = Shared Kernel · `CS` = Customer-Supplier · `ACL` = Anti-Corruption Layer · `OHS` = Open Host Service.

## Relationships table

| From → To | Pattern | Surface / contract |
|---|---|---|
| Workspace → all six others | **Shared Kernel** | Path-resolution variables: `${project}`, `${target_repo}`, `${task_id}`, `${agent_bus}` |
| Pipeline Authoring ↔ Runtime | **Shared Kernel** (`schema_version: 3`) | The live pipeline graph (now incl. fork/join parallel lanes, per-team `role` + bounded input `store`); validation designates the single **source** team (no forward inbound) and asserts **store reachability** from it (④a); hot-reload supported with save-validation preventing orphan in-flight tasks |
| Runtime → Review | **Customer-Supplier** | Runtime publishes "task reached a gate" events; Review consumes |
| Review → Runtime | **Conformist** | Review emits verdicts in Runtime's vocabulary (`approve` / `revise` / `reject`); Runtime owns the state machine |
| Runtime → Runners | **Anti-Corruption Layer** | Runners insulates Runtime from Claude's idiom (CLI flags, API params, stream-json shape) |
| Runners → Usage Telemetry | **Customer-Supplier** | Runners publishes usage events; Telemetry consumes |
| Claude Code transcripts → Usage Telemetry | **Conformist** | We adapt to Anthropic's `~/.claude/projects/**/*.jsonl` format; they don't care about us |
| Conversational Control → each of the six | **Customer (via OHS)** | Each supplier publishes a small Open Host Service of named app-tools; the terminal consumes only via those |

**Dependency direction is acyclic.** Workspace is the deepest kernel. Authoring, Runtime, and Review form the operational triangle. Runners + Telemetry sit below Runtime. Conversational Control is above (customer of all). No supplier depends on the customer; the customer doesn't depend on suppliers' internals.

## Aggregates and invariants — full tactical spine

### Pipeline Authoring

#### Aggregate: `Pipeline`

| | |
|---|---|
| **Root identity** | `pipeline_id` (string, unique per project) |
| **Composition** | `schema_version`, `teams[]`, `gates[]`, `escalations[]`, `forks[]`, `joins[]` |
| **Invariants protected by the root** | • Every `on_approve` / `on_revise` / `on_reject` resolves to an existing node id <br> • Every team is reachable from at least one entry point <br> • No node references itself <br> • Team ids are unique within the pipeline <br> • `schema_version` is present and supported by the current Runtime <br> • **A save must not orphan an in-flight task** (Pipeline rejects writes that would leave a Runtime task pointing at a removed node) |
| **Domain events** | `PipelineSaved(id, version, diff)` · `PipelineHotReloaded(id, prev_version, new_version)` |

### Runtime

#### Aggregate: `Task`

| | |
|---|---|
| **Root identity** | `task_id` (string, generated on inject) |
| **Composition** | `topic`, `current_stage`, `state`, `attempts`, `parent_artifact`, `review_artifact?`, `claim?` |
| **Invariants** | • `attempts ≤ max_attempts (3)` <br> • `state` follows allowed transitions: `queued → running → settled → (queued | gated | done | escalated)` <br> • At most one `claim` at a time (atomic; SQLite `BEGIN IMMEDIATE` or row-lock) <br> • When `state == gated`: only valid transitions are `→ queued` (verdict routes onward) or `→ escalated` <br> • A `done` task is immutable |
| **Domain events** | `TaskInjected` · `TaskClaimed` · `TaskSettled` · `TaskGated` · `TaskRouted` · `TaskFinalised` |

#### Aggregate: `WorkerPool` (per-team)

| | |
|---|---|
| **Root identity** | `team_id` (one pool per team in the active pipeline) |
| **Composition** | `workers[]` — each Worker has `pid`, `current_task_id?`, `model`, `effort`, `started_at` |
| **Invariants** | • `workers.count ≤ team.workers.max` <br> • `workers.count ≥ team.workers.min` (when not braked) <br> • No worker holds a task not assigned to its team |
| **Domain events** | `WorkerSpawned` · `WorkerCrashed` · `WorkerKilled` · `BrakeSet` · `BrakeReleased` |

> **Cross-reference, not composition.** Each Worker holds a `task_id`; each Task holds an optional `claim` referring to a worker. Two aggregates, joined by reference. One transaction can mutate one Task without locking the pool, and vice-versa.

> **④d cutover — the WorkerPool now drives the bounded-buffer engine.** At activation the `PipelineActivator` spawns **one generator loop** for the source team + **`workers.max` transformer loops** per other team (the pool is now real — `scale_team` reports the ceiling). Each loop polls its `engine` step (`generate_once` / `transform_once` / `fork_once`) for the project's **latest active run**, reserves-before-claim (backpressure), and on a settling step emits `task-changed`; on `try_finish_run` completion it emits `run-changed` + `usage-changed`. The old single-task `pool::process_one_claim` + `router::route` linear flow is **retired** (compiled-but-unwired, flagged `// DEAD: superseded by engine.rs`; a cleanup item). Gate verdicts route through `engine::apply_gate_verdict`.

#### Aggregate: `Run` (bounded-buffer assembly line — ④a foundation/migration 012; **LIVE ④d**)

| | |
|---|---|
| **Root identity** | `run_id` (one execution of a pipeline — the tree of work-items from a single Start) |
| **Composition** | `pipeline`, `project_id`, `generator_dry`, `completed`, `created_at` |
| **Invariants** | • **Completes exactly once** — `UPDATE runs SET completed=1 WHERE id=? AND completed=0` (single-writer guard; rows-affected==1 is the sole completer) <br> • The completion *precondition* (generator dry AND all stores empty AND no running/gated workers AND no open fan-out group) is checked by `engine::try_finish_run`; `try_complete` is only the guard |
| **Lifecycle** | **Start a run** (`runtime::api::start_run(topic: Option)`) creates the run + `ensure`s every team store at capacity; the generator loop kicks. `inject_topic` is a thin wrapper (terminal `/inject` still works). **Complete** is `try_finish_run` → emits `run-changed`. **Stop** = the brake. `latest_active_for_project` selects the run the loops drive. |
| **Module** | `runtime::run_store` + `runtime::engine` |

#### Aggregate: `Store` (bounded buffer — the backpressure boundary)

| | |
|---|---|
| **Root identity** | `(run_id, stage)` |
| **Composition** | `capacity`, `occupancy` |
| **Invariants** | • **`occupancy ≤ capacity`, always** — `UPDATE stores SET occupancy=occupancy+1 WHERE … AND occupancy<capacity` (rows-affected==1 = won a slot; 0 = full → backpressure) <br> • `release` guarded by `occupancy>0` (never negative) <br> • No count-then-act race — the conditional UPDATE is the single writer |
| **Module** | `runtime::store` |

> **Generator ledger** (not an aggregate root — a per-`(run_id, source-stage)` append-only key set in `runtime::generator_ledger`): the dedup + dry-detection source of truth. `record_keys` (INSERT OR IGNORE) returns the count of NEW keys (0 ⇒ a dry pass); `found_keys` is the already-found set handed to the generator each pass.
>
> **Work-item.** The `Task` aggregate gains `run_id` + `item_key` (the candidate key — lineage/dedup identity), additive/nullable on `tasks` (migration 012). As of the ④d cutover the worker pools drive the stores live; the board still groups Tasks (work-items ARE tasks), with run/store/pool indicators arriving in ④e.

### Review

#### Aggregate: `Artifact`

| | |
|---|---|
| **Root identity** | `artifact_path` (e.g. `artifacts/specs/T-040-v2.md`) |
| **Composition** | `versions[]` (each immutable once produced) · `comments[]` (anchored to a version + text span) |
| **Invariants** | • Versions are append-only — never rewritten in place <br> • An approved version is locked: no new comments may be added to it (you can comment on the *next* version when it arrives) <br> • Comment anchors must point at text that exists in their target version |
| **Domain events** | `ArtifactVersionProduced` · `CommentAdded` · `CommentResolved` · `ReviewVerdictEmitted(verdict, comments_bundled[])` |

**Comment-anchor policy across versions:** v1 of the app pins comments to the version they were authored against and shows a "v1 comments" sidebar when viewing v2 — operator gets context but anchors stay on v1. v1.1 upgrades to structured re-anchoring: the writer team writes `<!-- addressed: comment-id -->` markers in `vN+1` that re-attach the structured comment.

### Usage Telemetry

#### Aggregate: `UsageWindow` (singleton per project)

| | |
|---|---|
| **Root identity** | `project_id` (one window per project at a time) |
| **Composition** | rolling stream of `UsageEvent`s, configured `budget`, current `burn_rate`, `brake_threshold` |
| **Invariants** | • Events older than the window (5h) are not counted toward `window_total` <br> • Configured `budget > 0` <br> • `brake_threshold ∈ (0, 1]` |
| **Domain events** | `UsageRecorded` · `ThresholdBandCrossed` · `BrakeTriggered` · `BrakeAutoReleased` |

### Runners (ACL)

#### Aggregate: `Invocation`

| | |
|---|---|
| **Root identity** | `invocation_id` (generated per Claude call) |
| **Composition** | `task_id`, `team_id`, `model`, `effort`, `scope_settings_path`, `runner_kind`, `prompt`, `response`, `usage`, `status` |
| **Invariants** | • Once `status == started`, the request shape is locked — no late edits <br> • Must have a resolved scope settings file before launch <br> • On completion, must record `usage` <br> • On rate-limit: must emit `RateLimited` event and release the held Task back to Runtime |
| **Domain events** | `InvocationStarted` · `InvocationStreaming` (per chunk) · `InvocationCompleted` · `InvocationFailed` · `RateLimited` |

### Workspace

#### Aggregate: `Project`

| | |
|---|---|
| **Root identity** | `project_id` |
| **Composition** | `name`, `root_path`, `active_pipeline_id`, registered pipelines |
| **Invariants** | • `root_path` exists and is writable <br> • `active_pipeline_id` resolves to a pipeline in `pipelines/` <br> • `.agent-bus/` subdirectory exists and is gitignored |
| **Domain events** | `ProjectCreated` · `ProjectActivated` · `PipelineActivated` |

### Conversational Control

#### Aggregate: `Conversation` (singleton per project)

| | |
|---|---|
| **Root identity** | `project_id` (one ongoing conversation per project) |
| **Composition** | `turns[]` (alternating user/assistant) · `tool_calls[]` (request + result pairs) · `session_id`, `started_at`, `history_budget_tokens` |
| **Invariants** | • Turns strictly alternate user/assistant <br> • Every tool-call has a matching result (no orphan requests left dangling at idle) <br> • Total history tokens ≤ `history_budget_tokens` (truncate oldest user/assistant exchange when exceeded; preserve unresolved tool-calls) <br> • Sessions older than 24h get summarised on next launch |
| **Domain events** | `MessageSent` · `ToolCallDispatched` · `ToolCallResolved` · `ConversationTruncated` · `ConversationSummarised` |

## Event flow — one task end-to-end

A worked example, tracing a single task through the contexts via their events.

```mermaid
sequenceDiagram
    participant Op as Operator
    participant CC as Conversational Control
    participant RT as Runtime
    participant RN as Runners
    participant Cl as Claude
    participant RV as Review
    participant UT as Usage Telemetry

    Op ->>+ CC: "inject 03-scheduling"
    CC ->>+ RT: inject_topic(...)  [OHS app-tool]
    RT -->>- CC: TaskInjected(T-040)
    CC -->>- Op: "queued, watching"

    RT ->>+ RN: invoke(team=research, task=T-040, scope)
    RN ->>+ Cl: claude --print --stream-json ...
    Cl -->>- RN: stream chunks
    RN -->> UT: UsageRecorded(tokens, model)
    RN -->>- RT: InvocationCompleted(response)
    RT -->> RT: TaskSettled(verdict=approve)
    RT -->> RT: TaskRouted(research → spec-writers)

    Note over RT,RV: …several stages later…

    RT ->> RV: TaskGated(T-040, gate-1-spec)
    Op ->>+ RV: open drawer, read v1, add 2 comments, click revise
    RV -->>- RT: ReviewVerdictEmitted(revise, comments=[#1, #2])
    RT -->> RT: attempts++; route back to spec-writers
    RT -->> RT: TaskRouted(gate-1-spec → spec-writers, revise)

    Note over UT: rolling window passes 60%
    UT -->> Op: ThresholdBandCrossed(safe → warn)
    Note over Op,UT: meter colour shifts in topbar
```

The event flow demonstrates the relationship patterns in motion: ACL between Runtime and Runners; Customer-Supplier between Runtime and Review (TaskGated); Conformist back from Review to Runtime (verdict in Runtime's vocabulary); CS from Runners to Telemetry (usage); Customer relationship CC → Runtime via the `inject_topic` OHS app-tool.

## Anti-patterns guarded against

The model was designed to actively avoid these smells:

- **God aggregate.** A "Runtime" aggregate that owned both tasks and worker pools would lock everything every claim. Split into Task and WorkerPool, joined by reference.
- **Leaky boundary into Claude.** Without the Runners ACL, Claude's idiom (`stream-json`, `--max-thinking-tokens`, `usage.cache_creation_input_tokens`) would leak into Runtime invariants. Sealed at the ACL.
- **Accidental shared kernel via schema.** The Pipeline YAML *is* a shared kernel — declared deliberately with `schema_version` so changes are explicit.
- **Chatty coupling at the customer surface.** Conversational Control could have evolved into a god module that knew everything. Naming it a *customer* with explicit OHS surfaces from each supplier prevents the leak — adding a new app-tool is a deliberate cross-context change, not an internal helper.
- **Language-less context.** Every context has its own vocabulary (see DOMAIN.md Ubiquitous Language). No `DataManager` or `RuntimeProcessor` patterns.

## Open questions for v1.1 / v2

- **EscalationLog as its own aggregate?** Currently part of Runtime (escalations route to a `needs-human` sink). If escalation handling grows complex (re-tries, batching, notifications), promote to its own aggregate within Runtime or a new context.
- **Multi-project simultaneous open.** Each project has one Conversation. If we open multiple projects in tabs (v2), each tab gets its own conversation; cross-project commands aren't supported.
- **Audit log as a context?** All domain events go somewhere observable. Currently they're spread across SQLite tables. Could promote to an explicit Audit context (an event store) in v2.
- **Pipeline-level model defaults.** Should `pipeline.default_runner` and `pipeline.default_model` exist, with team-level overrides? Reduces config repetition for similar teams. v1.1 nice-to-have.

## References

- **Spec:** `~/assistant/Efforts/agent-bus-app/2026-06-22-agent-bus-app-design.md`
- **PRODUCT.md:** product positioning, anti-references, scene sentence
- **DESIGN.md:** visual tokens, components, anti-patterns
- **DOMAIN.md:** canonical roster + ubiquitous language for future `/ddd-council` runs
- **Predecessor spec:** `~/splose/agent-bus-design.md` (file-queue era; informs Runtime's task state machine)
