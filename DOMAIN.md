# DOMAIN.md

## Product

A local Tauri app for orchestrating multi-team Claude Code agent pipelines, with a UI-first replacement for the bash-supervised tmux predecessor. Single operator, local-only, distributable repo. Bundled DDD spec→plan→implement pipeline + a generic team-graph editor for users to design their own flows.

## Stack

- **Tauri 2.x** — Rust backend + native webview (WebKit/WebView2/WebKitGTK)
- **React 18 + TypeScript + Vite** — frontend
- **SQLite** via `tauri-plugin-sql` — runtime state (tasks, queue, usage)
- **YAML on disk** — pipeline definitions (`pipelines/*.yaml`)
- **Markdown on disk** — artifacts (specs, plans, critiques)
- **Two runner kinds** — `claude-cli` (wraps `claude --print`, inherits Claude Code plugins + subscription) and `anthropic-api` (direct SDK calls, per-team API key)

## Bounded contexts

Seven contexts. Six suppliers + one customer.

- **Pipeline Authoring** *(supplier)* — designing the team graph: nodes (teams, gates, escalations), edges (forward, revise, escalate), validation. Owns the pipeline YAML format with `schema_version`.
- **Runtime** *(supplier)* — executing a defined pipeline: spawning workers, claiming tasks, routing through stages, applying verdicts. Owns the task lifecycle state machine.
- **Review** *(supplier)* — reading artifacts (specs, plans, critiques), commenting inline, composing revisions, approving/rejecting at gates. Anti-clockwise from Runtime — the human-action surface.
- **Usage Telemetry** *(supplier)* — tracking token consumption from two sources (CC transcripts + our own runner output), computing the rolling 5h window, deciding when to set the brake.
- **Runners (ACL)** *(supplier)* — anti-corruption layer to Claude. Translates *(task + scope + prompt)* into Claude's idiom (CLI flags or API params) and Claude's responses back into our idiom (verdicts, artifacts, usage events).
- **LLM Chat (ACL)** *(supplier)* — anti-corruption layer for *multi-turn* Claude dialogue. Translates a stable `dialogue_id` + system framing + user message into Claude's chat idiom (`claude --print --output-format stream-json [--resume]`) and Claude's responses back into our idiom (assistant reply text + usage). Session continuity (`session_id`/`--resume`) is sealed inside the layer and never crosses the boundary. Consumed by Conversational Control (the god terminal) and Pipeline Authoring (the wizard's Design Session). Distinct from Runners, which stays the one-shot, verdict-shaped *worker* ACL.
- **Workspace** *(supplier)* — project setup, filesystem layout, path-resolution kernel. Provides the `${project}`, `${target_repo}`, `${task_id}` variables that every other context consumes.
- **Conversational Control** *(customer of all six)* — the god terminal. A persistent Claude session with tool-calls matching each supplier's published Open Host Service. Owns the Conversation aggregate (turns, tool-calls, history budget, session persistence).

## Domain experts

- **Architect** — speaks for *Pipeline Authoring*, *Runtime*, *Workspace* · strategic design; dependency direction; what changes on its own clock
  - vocabulary & rules: bounded context, anti-corruption layer, customer-supplier, shared kernel, conformist, dependency direction, no cycles
- **Engineer** — speaks for *Runtime*, *Runners*, *Review* (tactical) · invariants; state machines; the awkward transaction
  - vocabulary & rules: aggregate, invariant, transaction boundary, atomic claim, state transition, idempotency, crash recovery
- **AI Engineer** — speaks for *Runners*, *Usage Telemetry*, *Conversational Control* · LLM behaviour; token economics; prompt fragility
  - vocabulary & rules: model, effort (thinking budget), context window, prompt cache, stream-json, tool-call protocol, rate limit, completion vs streaming
- **Conversational UX Designer** — speaks for *Conversational Control* · interaction design for chat surfaces
  - vocabulary & rules: turn, intent, ambiguity resolution, tool-call rendering, history budget, command palette, slash-commands
- **Operator** — speaks for *Pipeline Operation* (the lived workflow across contexts) · the user; daily rhythm; what gets in the way
  - vocabulary & rules: inject, brake, scale, gate, revise, approve, reject, needs-human, worktree
- **Tim** — canon authority on all domain facts. The room pauses and asks Tim when no expert can settle a question from evidence or general knowledge.

> **Cross-expert tension to expect:** Engineer and AI Engineer will disagree on how strictly to enforce invariants when LLM outputs are slippery. Engineer wants Task invariants enforced at the DB layer; AI Engineer points out that Claude's response shape can vary by model and that strict validation makes the system brittle. The Anti-Corruption Layer (Runners context) is exactly where this tension resolves — Runners absorbs the slipperiness so Runtime's invariants stay clean.

## Lens

default: **strategic** · **design** · **workshop**

Reasoning: this project is *being designed* (no code yet). The default verb is `map` or `vet` (against the spec). Workshop register because the brainstorm benefits from showing the friction. Once code lands, the default shifts to **critique** + **brief** for autonomous runs (same pattern as the splose-monorepo lens).

## Focus

**Pipeline Authoring** and **Runtime** — these two are the largest contexts and the most invariant-rich. Most early implementation work touches one of them. Move to other contexts (Review, Telemetry, Runners) once the core lifecycle is stable.

## Ubiquitous language

Initial entries — extracted per-context as `/ddd-council language` is run on each. See `docs/context-map.md` for the full strategic + tactical model.

### Cross-context (the kernel)
- **Project** — a workspace with a root directory, an active pipeline, and a registered set of pipelines
- **Path variables** — `${project}` (project root), `${target_repo}` (the splose-monorepo or other repo being modified), `${task_id}` (the in-flight task), `${agent_bus}` (alias for `${project}` for migration-era code)

### Pipeline Authoring
- **Pipeline** — the graph: a versioned (`schema_version`) collection of teams, gates, escalations
- **Team** — a node in the graph with one prompt, one scope, one runner config (a team may inherit the pipeline-level default and override fields selectively — R5; the *resolved* team always has exactly one fully-specified runner config)
- **Pipeline defaults / effective runner config** — `Pipeline.defaults` (`default_runner` / `default_model` / `default_effort`) supply runner config that teams inherit when they omit their own (R5). The **effective runner config** is a team's runner after the pipeline defaults are overlaid; Pipeline Authoring resolves it at load (`resolve.rs`) so Runtime always consumes a fully-specified `RunnerConfig` and never learns about defaults.
- **Gate** — a node where execution pauses until the operator approves/revises/rejects
- **Escalation** — a sink for tasks that can't proceed (≥3 revises or explicit reject)
- **Route** — an edge: `on_approve` / `on_revise` / `on_reject` pointing at another node
- **Fork** — a node that fans one task out into parallel lanes (routes stay single-target; the multiplicity is the fork's `lanes`)
- **Join** — a barrier node that waits for all lanes, then continues — all must approve, else needs-human
- **Lane** — a linear team chain between a fork and its join; named to avoid colliding with git/worktree *branch*
- **Design Session** — an ephemeral, AI-assisted authoring dialogue that produces a pipeline, conducted over the LLM Chat ACL (one `dialogue_id` per wizard step: `<session>:<step>`). Distinct from the terminal's `Conversation` aggregate — it has no persistence and lives in frontend state + an in-memory chat session for the duration of the wizard. Drives the 5-step new-project wizard (basics → teams → responsibilities → wiring → review).
- **DraftPipeline** — an in-progress, not-yet-valid pipeline the wizard edits; distinct from the validated `Pipeline` aggregate. Best-effort validation surfaces issues live during editing; only a `DraftPipeline` that passes **hard** validation (`validate.rs`) at create becomes a `Pipeline`. Prompt text is held inline; Pipeline Authoring serializes it (YAML + per-team prompt files) and Workspace writes it. The wizard surfaces **best-effort validation** live during steps 2–4 (W1), supports a full **per-team advanced panel** (model, effort preset, tools, scope reads/writes — W2), and authors **human-review gates** in the Wiring step (`DraftPipeline` carries gates; `to_pipeline` emits them; the no-gate-inside-a-lane parallel-flow rule is honored — W3).

### Runtime
- **Task** — a unit of work moving through the pipeline; identified by `task_id`
- **Stage** — the team or gate the task is currently at
- **Claim** — the atomic act of a worker taking a task from inbox to working
- **Settle** — the worker finishing; emits a verdict event
- **Stream (live log)** — while a worker runs, its invocation may **stream** display-only log deltas in addition to its terminal **Settle**. Streaming is a side channel for live display (R4); **Settle** remains the single verdict moment. The deltas never influence settle/route.
- **Verdict** — `approve` | `revise` | `reject`
- **Attempts** — counter incremented on revise; capped at 3
- **Brake** — system-wide flag halting new claims (in-flight workers complete)
- **Fan-out group** — the `FanOutGroup` aggregate (root `group_id`) owning the *completes-exactly-once* barrier invariant; the lane sibling Tasks reference it
- **Early-cancel** — an opt-in per-join policy (`Join.cancel_on_reject`, P2): when one lane fails (reject / revise-cap), the fan-out group resolves to needs-human IMMEDIATELY and its outstanding lane tasks are cancelled, instead of waiting for the full barrier. Owned by the `FanOutGroup` aggregate; default off (full-barrier).

### Review
- **Artifact** — a markdown document produced by a team (spec, plan, critique, review report). Versioned (`v1`, `v2`, …); immutable per version
- **Comment** — a note anchored to a span in a specific artifact version
- **Thread** — a comment + its resolutions across versions
- **Send back** — the act of bundling comments + optional direction and emitting a `revise` verdict

### Usage Telemetry
- **Usage event** — one `(ts, team?, task?, model, input/output/cache tokens)` record
- **Window** — the rolling 5h period for subscription-based runners
- **Budget** — configured threshold for the window (per subscription tier)
- **Burn rate** — tokens per minute, 1-min average
- **Threshold band** — `safe` (<60%), `warn` (60–85%), `hot` (≥85%), `braked`

### Runners (ACL)
- **Invocation** — one Claude call: CLI subprocess or API request
- **Runner kind** — `claude-cli` (subscription) or `anthropic-api` (key)
- **Effort** — thinking-token budget: `off` (0), `standard` (1024), `extended-low` (8192), `extended-high` (32000), `custom`
- **Scope** — the per-invocation `settings.json` defining permission allow/deny
- **Log delta** — a display-only assistant-prose fragment the *streaming* worker invocation (`invoke_stream`) forwards via a `LogSink` (`Box<dyn Fn(&str)>`) as it runs; carries no verdict/artifact and never crosses the ACL as stream-json. Mirrors LLM Chat's prose delta (`DeltaSink`), kept as a separate type because the two ACLs are distinct (R4).

### Workspace
- **Project root** — the directory the user picked; contains `pipelines/`, `prompts/`, `artifacts/`, `worktrees/`, `.agent-bus/`
- **Project write surface** — `write_project_pipeline` writes a project's pipeline YAML + per-team prompt files (`prompts/<team>.md`) under the (already `~`-expanded) project root, path-scoped with the `resolve_under_root` escape guard — the counterpart to `read_artifact`. (The bundled-**Template** instantiation path was dropped in sub-project 3; the wizard writes YAML directly. Templates may return as wizard seeds in v1.1.)

### Conversational Control
- **Conversation** — the persistent dialogue with the terminal's Claude session
- **Turn** — one user message + one assistant response (with embedded tool-calls)
- **App-tool** — a named operation a supplier publishes to the terminal (`inject_topic`, `approve_gate`, etc.)
- **Tool-call** — a request from Claude to invoke an app-tool, with structured args
- **History budget** — token cap on the conversation; oldest exchanges drop out

## Shared kernels

Two deliberate shared kernels (both small, both stable, both with explicit ownership rules):

1. **Workspace path-resolution kernel** — the path variables (`${project}`, `${target_repo}`, `${task_id}`, `${agent_bus}`) every context consumes when locating files. Owned by the Workspace context; published as a small API.
2. **`agent_bus_core` cross-context primitives** — a dedicated Rust crate holding ID newtypes (`TaskId`, `TeamId`, `PipelineId`, `ProjectId`, `ArtifactPath`), cross-context enums (`Verdict`, `RunnerKind`, `EffortMode`), and the OHS tool protocol (`ToolSpec`, `ToolCallRequest`, `ToolCallResult`). No owner-context; this kernel is owned by the architecture itself. Depends on nothing project-internal. Every context depends on it.

These are *deliberate* shared kernels — explicit, named, small, with documented invariants. They do not trip the accidental-shared-kernel detector because the detector config (below) marks both as kernels.

## Notes for the detector

The project hasn't been built yet. Once code lands, the detector config (`ddd-council.json`) maps the seven contexts to source paths and declares the two shared kernels. Initial mapping suggestion (paths relative to the future repo root):

```json
{
  "schema_version": 1,
  "kernels": {
    "agent_bus_core": { "module": "src-tauri/agent_bus_core", "paths": ["src-tauri/agent_bus_core/**"] }
  },
  "contexts": {
    "pipeline-authoring": { "module": "src/authoring",          "paths": ["src/authoring/**", "src-tauri/src/authoring/**"], "publicModules": ["api"] },
    "runtime":            { "module": "src/runtime",            "paths": ["src/runtime/**",   "src-tauri/src/runtime/**"],   "publicModules": ["api"] },
    "review":             { "module": "src/review",             "paths": ["src/review/**",    "src-tauri/src/review/**"],    "publicModules": ["api"] },
    "usage-telemetry":    { "module": "src/usage",              "paths": ["src/usage/**",     "src-tauri/src/usage/**"],     "publicModules": ["api"] },
    "runners":            { "module": "src/runners",            "paths": ["src-tauri/src/runners/**"],                       "publicModules": ["api"] },
    "workspace":          { "module": "src/workspace",          "paths": ["src/workspace/**", "src-tauri/src/workspace/**"], "publicModules": ["api"] },
    "conversational-control": { "module": "src/terminal",       "paths": ["src/terminal/**", "src-tauri/src/terminal/**"],   "publicModules": ["api"] }
  }
}
```

Place this file at the repo root once the project is created. The Rust crate structure should mirror these context boundaries — each context a Cargo workspace member with its own `api` submodule. The `agent_bus_core` crate is the kernel; every context depends on it; it depends on nothing.
