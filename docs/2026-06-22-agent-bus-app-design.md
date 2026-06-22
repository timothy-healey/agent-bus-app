---
id: 2026-06-22-agent-bus-app-design
title: Agent Bus App — System Design
status: draft
date: 2026-06-22
owner: tim.healey@splose.com
register: product
---

# Agent Bus App — System Design

A local Tauri app that replaces the bash-supervised tmux-based agent-bus (`~/agent-bus/`) with a UI-first orchestrator for multi-team Claude Code agent pipelines. The user defines teams (each team = prompt + scope + runner), connects them into a graph (forward edges, revise loops, gates, escalation routes), and the app runs workers, surfaces gated decisions, lets the user review artifacts and bounce them back with comments, and tracks per-task and rolling-window usage.

The bundled default pipeline is a DDD spec → plan → implement flow (the same pattern explored in `~/splose/agent-bus-design.md`). Users can edit it or design their own from the canvas.

## Goals

- **Replace 8-pane tmux** with a single UI that's easier to glance, easier to act on, and pleasant to read documents in.
- **Generalise the pattern** — any user with any project can clone the repo and define their own team graph; the DDD pipeline becomes a template.
- **Visibility over rolling usage** — surface aggregate context-window consumption from the topbar so the user can pace themselves against subscription limits.
- **Conversational control** — a god terminal at the bottom lets the user issue commands in natural language (inject, approve, brake, scale) without learning button layouts. Buttons stay as the canonical surface for canonical actions.
- **Restrained, deliberate aesthetic** — full-mono warm dark by default; warm light alternate. One accent (ochre) for "needs you" + primary actions. Refer to `DESIGN.md` for tokens.
- **Earned familiarity** — Kanban for the main view (predictable affordance), list view for filtering, pipeline editor for graph topology. No reinvented UX for standard tasks.

## Non-goals

- **Multi-user / team collaboration.** v1 is single-operator local. Anyone can clone the repo and use it for their project, but each install is single-user.
- **Hosted SaaS.** No cloud component. All state local.
- **Mobile.** Tauri desktop only (macOS, Linux, Windows).
- **Replacing the file-queue agent-bus.** The two systems are independent. Users on the legacy `~/agent-bus/` can keep using it; the new app is a standalone product, not a migration target. Some patterns transfer; no code does.
- **Auto-merging or auto-pushing branches.** The user pushes after reviewing the worktree.
- **Custom model fine-tuning / inference.** Uses Claude through `claude --print` or the Anthropic SDK.

## Architecture

- **Tauri 2.x** as the app shell. Rust backend + WebKit (macOS) / WebView2 (Windows) / WebKitGTK (Linux) renderer.
- **React 18 + TypeScript + Vite** as the frontend.
- **Tauri commands** for IPC (typed Rust functions called from TS via `invoke`).
- **Tauri events** for backend-pushed updates (worker stdout streaming, usage updates, task state changes).
- **SQLite via `tauri-plugin-sql`** for orchestration state (tasks, queue, usage logs).
- **File system** for artifacts (specs, plans, reviews live as markdown files under the project's bus directory).

### Process model

```
┌────────────────────────────────────────────────────────────┐
│ Tauri main process (Rust)                                  │
│ ─ WorkerPool service (tokio task per worker)               │
│ ─ Pipeline router (routes outboxes, manages cycles)        │
│ ─ Usage Telemetry watcher (transcripts + SDK tail)         │
│ ─ Scope policy (per-invocation settings.json generation)   │
│ ─ Tauri command surface (per-context api submodules)       │
│ ─ Tauri event emitter (worker.output, task.changed, usage) │
└──────────────┬─────────────────────────────────────────────┘
               │ subprocess spawns (one per active worker)
               ▼
┌──────────────────────────────────────┐  ┌─────────────────────────────────┐
│ claude --print (CLI runner)          │  │ Anthropic SDK call (API runner) │
│ ─ stream-json output → parsed live   │  │ ─ async via reqwest             │
│ ─ inherits user's CC auth            │  │ ─ uses ANTHROPIC_API_KEY        │
│ ─ has access to installed plugins    │  │ ─ no CC plugins                 │
└──────────────────────────────────────┘  └─────────────────────────────────┘
               ▲                                       ▲
               └──── usage tracked in real-time ───────┘
                              │
                              ▼
                ┌─────────────────────────┐
                │ SQLite usage_log tables │
                └─────────────────────────┘
```

The renderer (React) is a view layer over Tauri commands; never does business logic, never spawns subprocesses, never touches user files directly.

### Tauri command organisation

Each context's `api` submodule defines its own Tauri commands; `src-tauri/src/lib.rs` only registers them. Concretely, the seven Rust crates (one per bounded context) each expose:

```rust
// e.g. src-tauri/src/runtime/api.rs
#[tauri::command]
pub async fn inject_topic(...) -> Result<TaskId, RuntimeError> { ... }

#[tauri::command]
pub async fn approve_gate(...) -> Result<(), RuntimeError> { ... }

pub fn register(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        inject_topic, approve_gate, // …
    ])
}
```

And `lib.rs` aggregates registrations:

```rust
fn build_app() -> tauri::App {
    let mut builder = tauri::Builder::default();
    builder = pipeline_authoring::api::register(builder);
    builder = runtime::api::register(builder);
    builder = review::api::register(builder);
    builder = usage_telemetry::api::register(builder);
    builder = runners::api::register(builder);
    builder = workspace::api::register(builder);
    builder = conversational_control::api::register(builder);
    builder.build(...).run(...)
}
```

This makes each context's command surface its **Open Host Service** at the language level — readable, ownership-clear, and impossible to accidentally cross context lines when adding a new command.

### Cross-context primitives (shared kernel)

The OHS examples above reference types — `TaskId`, `ToolSpec`, `Verdict`, etc. — that several contexts use. These primitives live in a small dedicated Rust crate, `agent_bus_core`, treated as a **shared kernel** alongside Workspace's path-resolution kernel (see DOMAIN.md → Notes for the detector).

```
src-tauri/
├── agent_bus_core/              # shared kernel crate; depends on nothing project-internal
│   └── src/
│       ├── ids.rs               # TaskId, TeamId, PipelineId, ProjectId, ArtifactPath
│       ├── verdict.rs           # Verdict enum (approve | revise | reject)
│       ├── runner.rs            # RunnerKind enum + EffortMode enum
│       └── tool_protocol.rs     # ToolSpec, ToolCallRequest, ToolCallResult
├── pipeline_authoring/          # depends on agent_bus_core
├── runtime/                     # depends on agent_bus_core
├── review/                      # depends on agent_bus_core
├── usage_telemetry/             # depends on agent_bus_core
├── runners/                     # depends on agent_bus_core
├── workspace/                   # depends on agent_bus_core
├── conversational_control/      # depends on agent_bus_core
└── lib.rs                       # composition root
```

**Contents of `agent_bus_core`:**

| Module | Types |
|---|---|
| `ids` | `TaskId(String)`, `TeamId(String)`, `PipelineId(String)`, `ProjectId(String)`, `ArtifactPath(PathBuf)` — newtype wrappers for type-safe IDs |
| `verdict` | `Verdict { Approve, Revise, Reject }` — used by Runtime (consumer) and Review (producer) |
| `runner` | `RunnerKind { ClaudeCli, AnthropicApi }`, `EffortMode { Off, Standard, ExtendedLow, ExtendedHigh, Custom(u32) }` — used by Pipeline Authoring (defines), Runners (implements), Runtime (consumes) |
| `tool_protocol` | `ToolSpec { name, schema, supplier_context }`, `ToolCallRequest`, `ToolCallResult` — used by Conversational Control (consumer) and every supplier (producer of its OHS tools) |

**Hard rules for `agent_bus_core`:**

- **Depends on nothing project-internal.** Only `std`, `serde`, `serde_json`, and Tauri's IPC types. No cycles possible.
- **Stable.** Changes here are breaking changes for every context. Reviewed accordingly.
- **No business logic.** Just primitive types, newtype wrappers, and small enums. No state, no I/O, no side effects.
- **No re-exports from contexts.** Contexts depend on the kernel, not vice versa. Adding a type means adding it here, not somewhere else.

> 📖 *Why a second shared kernel?* The DDD law against accidental shared kernels prohibits *unowned* sharing — types that drift in two places at once. A *deliberate* shared kernel with an owner and explicit rules is the correct pattern when multiple contexts genuinely need the same primitives. The `ddd-council` `init` verb's detector config will mark `agent_bus_core` as a kernel module so cross-context imports from it don't trip the accidental-shared-kernel detector.

## Data model

### Filesystem layout

```
~/<user-chosen-project-dir>/         # the "project root" — picked at project creation
├── pipelines/                       # graph definitions
│   ├── ddd-spec-plan-impl.yaml      # bundled template (copied here on init)
│   └── <custom>.yaml
├── prompts/                         # team operating prompts (markdown)
│   ├── research.md
│   └── …
├── artifacts/                       # produced by workers
│   ├── analyses/<id>.md
│   ├── specs/<id>-v<N>.md
│   ├── plans/<id>-v<N>.md
│   └── reviews/<id>-<stage>-v<N>.md
├── worktrees/<id>/                  # implementer worktrees
└── .agent-bus/                      # app's local state, gitignored
    ├── state.db                     # SQLite (tasks, queue, usage)
    ├── runtime/                     # per-invocation settings.json files
    └── logs/                        # team logs (debug)
```

The user picks the project root when creating a project; the wizard `mkdir`s the structure. Everything under `.agent-bus/` is gitignored by default.

### SQLite schema

```sql
-- Pipeline definitions are stored as YAML files on disk; SQLite mirrors only the
-- runtime state.

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  root_path TEXT NOT NULL,
  active_pipeline TEXT,
  created_at INTEGER
);

CREATE TABLE tasks (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  pipeline TEXT NOT NULL,
  topic TEXT NOT NULL,
  target_repo TEXT,
  target_scope TEXT,
  current_stage TEXT NOT NULL,      -- team id or gate id
  state TEXT NOT NULL,              -- queued | running | gated | revising | needs_human | done | braked
  attempts INTEGER DEFAULT 1,
  parent_artifact TEXT,
  review_artifact TEXT,
  created_at INTEGER,
  updated_at INTEGER,
  FOREIGN KEY(project_id) REFERENCES projects(id)
);

CREATE TABLE comments (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL,
  artifact_path TEXT NOT NULL,
  anchor_text TEXT,                 -- the quoted span
  anchor_offset INTEGER,            -- char offset for stable re-anchoring on revision
  note TEXT NOT NULL,
  created_at INTEGER
);

CREATE TABLE workers (
  id TEXT PRIMARY KEY,
  team_id TEXT NOT NULL,
  task_id TEXT,                     -- null when idle
  pid INTEGER,                      -- subprocess pid
  started_at INTEGER
);

-- Usage attributed to our own worker invocations (per-team breakdown source)
CREATE TABLE worker_usage_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  team_id TEXT NOT NULL,
  task_id TEXT,
  model TEXT NOT NULL,
  input_tokens INTEGER DEFAULT 0,
  output_tokens INTEGER DEFAULT 0,
  cache_creation INTEGER DEFAULT 0,
  cache_read INTEGER DEFAULT 0
);

-- Mirror of Claude Code's transcript usage (window-total source)
CREATE TABLE cc_usage_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  message_id TEXT NOT NULL UNIQUE,
  model TEXT,
  input_tokens INTEGER DEFAULT 0,
  output_tokens INTEGER DEFAULT 0,
  cache_creation INTEGER DEFAULT 0,
  cache_read INTEGER DEFAULT 0
);

-- Conversation aggregate (god-terminal session). One per project. Persists
-- across app restarts so the 24h-summarisation invariant is enforceable
-- and the terminal's "always-present" continuity claim survives a relaunch.
CREATE TABLE conversations (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  last_message_at INTEGER NOT NULL,
  history_json TEXT NOT NULL,            -- serialised turns (alternating user/assistant)
  summary_of_prior_sessions TEXT,        -- written when prior sessions age past 24h
  FOREIGN KEY(project_id) REFERENCES projects(id)
);

CREATE INDEX idx_tasks_state ON tasks(state);
CREATE INDEX idx_worker_usage_ts ON worker_usage_log(ts);
CREATE INDEX idx_cc_usage_ts ON cc_usage_log(ts);
CREATE INDEX idx_conversations_project ON conversations(project_id);
```

### Pipeline definition

A pipeline YAML file defines the graph:

```yaml
id: ddd-spec-plan-impl
name: DDD Spec → Plan → Implement
description: Domain-driven design pipeline with two human gates.

teams:
  - id: research
    name: Research
    prompt: prompts/research.md
    runner:
      kind: claude-cli
      model: claude-opus-4-7
      effort:
        mode: extended-high
    scope:
      reads: ["${target_repo}", "${project}/artifacts/analyses"]
      writes: ["${project}/artifacts/analyses", "${target_repo}/docs/critique-*.md"]
      tools: [Read, Write, Edit, Glob, Grep, "Bash(ls:*)", "Bash(rg:*)", "Bash(git diff:*)", Skill]
    outputs:
      on_approve: spec-writers
      on_revise: null        # research has no upstream
      on_reject: needs-human
    workers:
      default: 1
      max: 3

  # Note: this example shows the anthropic-api runner kind to illustrate
  # per-team runner choice — but v1 ships only claude-cli. Treat anthropic-api
  # as a forward-pointer for v1.1; in v1, this team's runner.kind would be
  # claude-cli with the same model/effort fields.
  - id: spec-writers
    name: Spec Writers
    prompt: prompts/spec-writers.md
    runner:
      kind: anthropic-api
      model: claude-sonnet-4-6
      effort:
        mode: extended-low
      api_key_env: ANTHROPIC_API_KEY
    scope: { … }
    outputs:
      on_approve: spec-reviewers
      on_revise: research
      on_reject: needs-human

  # … other teams …

  # spec-reviewers' outputs route to the gate, not directly to plan-writers
  - id: spec-reviewers
    outputs:
      on_approve: gate-1-spec
      on_revise: spec-writers
      on_reject: needs-human
    # … runner, prompt, scope, workers …

gates:
  - id: gate-1-spec
    label: Gate 1 — Spec Approval
    downstream: plan-writers       # where approved files route

  - id: gate-2-plan
    label: Gate 2 — Plan Approval
    downstream: implementers

escalations:
  - id: needs-human
    triggers: ["attempts >= 3", "verdict == reject"]
```

**Routing model.** Teams' `on_approve` / `on_revise` / `on_reject` each point at either a team id, a gate id, or an escalation sink id. Gates and escalations are first-class node types — they appear in the graph alongside teams. The gate has only a `downstream` field (where approved files go); its `upstream` is implied by which team's `on_approve` points to it. Validation on save: every output reference must resolve to a known node; no orphan nodes; no node points to itself.

### Team prompt shape

`prompts/<team-id>.md` is a markdown file consumed as Claude's `--append-system-prompt` (or `system` for API runners). It owns the team's *behaviour*; the team YAML owns the *plumbing* around it. No structural constraints — just markdown.

## Worker model

### The WorkerPool service

A long-lived Tokio task in the Rust backend. Owns:

- A `HashMap<TeamId, Vec<WorkerHandle>>` of active workers.
- A scaling controller that observes inbox depth and consults the team's `workers.max` to decide whether to spawn more (manual scale via UI or terminal still possible).
- Lifecycle hooks: spawn, kill (graceful + SIGTERM-after-timeout), restart-on-crash, drain-on-brake.

### Per-worker lifecycle

```
   ┌─────────────────────────────────────┐
   │ 1. POLL: SELECT next task for team  │
   │    (atomic UPDATE state=running)    │
   └──────────────┬──────────────────────┘
                  │
   ┌──────────────▼──────────────────────┐
   │ 2. PREPARE: scope enforcer writes   │
   │    runtime/<task_id>-<team>.settings│
   │    .json (permissions allow/deny)   │
   └──────────────┬──────────────────────┘
                  │
   ┌──────────────▼──────────────────────┐
   │ 3. SPAWN: subprocess (CLI or SDK)   │
   │    stdout → parsed stream-json      │
   │    stderr → captured for diagnostics│
   └──────────────┬──────────────────────┘
                  │
   ┌──────────────▼──────────────────────┐
   │ 4. STREAM: each chunk → tauri.event │
   │    "worker.output" for live render  │
   │    + worker_usage_log insert        │
   └──────────────┬──────────────────────┘
                  │
   ┌──────────────▼──────────────────────┐
   │ 5. SETTLE: read outbox, parse       │
   │    verdict, update task row,        │
   │    delete settings.json             │
   └──────────────┬──────────────────────┘
                  │
   ┌──────────────▼──────────────────────┐
   │ 6. ROUTE: Pipeline router moves the │
   │    task per outputs map; bumps      │
   │    attempts on revise               │
   └─────────────────────────────────────┘
```

**Invocation crash policy (v1).** Invocations are kept in process memory only — no SQLite persistence per Invocation. On a Runtime crash mid-invocation, the in-flight partial response is lost; on next launch, the Task's `claim` is released (since no live worker holds it) and the Task re-enters its team's inbox to be retried by a fresh Invocation. Aggregate invariants on Invocation apply only while the process is alive. v1.1 may persist Invocation start/completion records for audit, but v1 trusts Task-level claim recovery for correctness.

### Runners

Two implementations of the `WorkerRunner` Rust trait:

```rust
trait WorkerRunner: Send + Sync {
    async fn invoke(
        &self,
        team: &Team,
        task: &Task,
        scope: &ScopeSettings,
    ) -> Result<RunnerOutput, RunnerError>;
}

struct ClaudeCliRunner { /* … */ }
struct AnthropicApiRunner { /* … */ }
```

**`ClaudeCliRunner`** spawns `claude --print --output-format stream-json --append-system-prompt "$(cat prompt.md)" --settings <runtime/...settings.json> --add-dir <…> --permission-mode acceptEdits --model <id> --max-thinking-tokens <effort.budget> "<user message>"`. Parses stream-json line-by-line; emits Tauri events; persists usage to `worker_usage_log`.

**`AnthropicApiRunner`** uses the official Anthropic Rust SDK. Builds a request from the team's prompt + scope-derived tool config + the task content. Streams responses; same event/log shape.

Trait abstraction means the rest of the system never branches on runner kind.

### Scope enforcement

Per the `team-scope.html` mockup. For each worker invocation, the Scope policy module:

1. Resolves variables (`${target_repo}`, `${project}`, `${task_id}`) against the current task.
2. Writes `runtime/<task_id>-<team>-<ts>.settings.json` with `permissions.allow` / `permissions.deny` patterns (CLI runner) or constructs tool restriction config (API runner).
3. Passes the settings path + `--add-dir` flags to the subprocess (CLI) or applies the equivalent client-side allow-listing (API).
4. Deletes the runtime file when the worker settles.

A future v2 hardens this with `sandbox-exec` / `bubblewrap`. v1 trusts Claude's permission system.

## UI surfaces

All visual choices documented in `DESIGN.md`. This section describes information architecture and behavioural detail.

### Topbar

Persistent across all views. Left to right:

- **Brand** (mono): `● agent bus`
- **Active pipeline pill** — clicking opens pipeline picker (project-scoped).
- **Spacer**
- **Usage meter widget** — bar + % + window mark + burn rate. Hover for breakdown tooltip. Three colour states (safe/warn/hot) plus braked state.
- **Brake state** — `● brake off` / `● brake on (reason: rate-limit)`. Clicking toggles.

### View switcher

Below the topbar. Three tabs:

- **board** (default) — Kanban with one column per team + special columns for gates.
- **list** — table of all tasks with filter pills + search. Power-user filtering.
- **pipeline** — node-graph editor for the active pipeline.
- **settings** — right-aligned; opens preferences.

### Board (Kanban)

- One lane per team, in declared graph order; gate lanes interleaved at the right position.
- Horizontal scroll for >8 teams.
- Cards as documented in `DESIGN.md` (id · state pill · title · meta row with age · cost · worker · attempts).
- Gate lanes subtly accent-tinted.
- Click any card → drawer slides in from the right.

### Card detail drawer

Slide-over from the right, 60% of viewport width (clamped 600–960px). Four tabs:

1. **Artifact** — rendered markdown of the artifact this card produced. Inline selection → comment popover. Right rail shows saved comments. Action bar at the bottom: reject · revise · approve.
2. **Live log** — stream-json output, auto-scrolling while running. Goes static when done.
3. **Review** — for cards that have gone through a reviewer: the review artifact rendered, findings list with status (open / resolved / deferred). The Gate cards' approve/revise/reject lives here too.
4. **Lineage** — upstream artifacts (topic → analysis → spec → plan → impl). Click any to side-by-side compare.

### Revise flow

See `assets/designs/05-revise-flow-final.png` and `assets/html/revise-flow-v2.html` for the six-step UX. Summary:

1. Plan lands at gate; banner pulses; card pulses in its lane.
2. Click card; drawer opens with artifact tab.
3. Select text inline; popover composes a comment.
4. Comments collect in the right rail with numbered markers anchored inline.
5. Click `revise & send back`; expanded panel shows comment summary + optional overall-direction textarea.
6. Send. Card animates back to writer lane in revise-purple. Attempts bumps.

### List view

Filter pills (all / needs you / running / revising / braked / cost > 100k) + free-text search. Table columns: id · topic · stage · state · age · tokens · attempts · last-update. Rows have the same hover-to-act affordances as cards. `needs-you` rows have a subtle ochre left edge.

### Pipeline editor

Dot-grid canvas. Drag nodes from right sidebar palette. Click an edge handle and drag to a target to connect. Selected node opens an inline config panel (the same 6 fields the team YAML carries). Forward edges straight; revise back-edges curved + dashed; escalation routes dashed in danger colour.

Save → writes YAML to `pipelines/<id>.yaml` and validates. Invalid graphs (unresolved references, orphan nodes) refuse to save with a clear error.

### God terminal

Always-present at the bottom. Collapsible to a thin bar. When expanded:

- Top: `● claude` indicator + context line (e.g. `context: full pipeline + 8 teams`).
- Middle: scrollable conversation. User messages, Claude responses, **tool-call chips** inline (`→ inject_topic T-042 ✓`).
- Bottom: input row with `›` prompt + hints (`↑ history`, `⌘K commands`).

Claude has app-specific tools matching the canonical actions:

| Tool | Effect |
|---|---|
| `inject_topic` | Drop a topic file into research's inbox |
| `approve_gate` | Move file from gate's awaiting → approved |
| `reject_gate` | Move file from gate's awaiting → rejected |
| `revise_gate` | Send file back to writer with provided comments |
| `brake_on` / `brake_off` | Toggle the brake |
| `scale_team` | `+1` / `-1` / `set N` workers for a team |
| `summarise_artifact` | Read a spec/plan/review and return a 3-line summary |
| `query_state` | Return current pipeline state for the terminal's prompt context |
| `watch_outbox` | Set up a one-shot trigger (used to schedule next-action) |

Implementation: the Conversation (god-terminal UI) is itself a `claude-cli` (or `anthropic-api`) session, persistent across the app lifetime. The session has the pipeline definition + recent state injected as system context on each turn. App-tools are exposed via Claude's tool-use mechanism — the Rust backend handles tool calls by dispatching to the same internal functions buttons would.

**Tool catalog ownership.** The terminal's available tool catalog is the union of every supplier's published app-tools. It is **constructed at startup** by querying each supplier's `api::tools() -> Vec<ToolSpec>` function:

```rust
// e.g. src-tauri/src/runtime/api.rs
pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec { name: "inject_topic",  schema: ... },
        ToolSpec { name: "approve_gate",  schema: ... },
        ToolSpec { name: "scale_team",    schema: ... },
        // …
    ]
}
```

The catalog is **in-memory, read-only after construction, and never persisted**. Adding a new app-tool requires touching exactly two places: the supplier's `tools()` function and the terminal's tool-call UI affordance. This is deliberate — adding a new app-tool *is* a cross-context change, and the build-time aggregation makes that explicit. No god module, no stale-cache risk, no runtime mutable registry.

### Settings

Preferences pane. Sections:

- **General** — theme (system / light / dark), font family override, language.
- **Usage** — subscription tier (Pro / Max / API / custom) → sets the assumed 5h window budget for the meter. Per-team default model/effort overrides.
- **Runners** — `ANTHROPIC_API_KEY` field (stored in OS keychain via `tauri-plugin-stronghold`). Claude Code installation path detection.
- **Git** — confirm worktree base directory; warn if `~/.ssh/` is referenced anywhere in team scopes.
- **Projects** — list of known projects + add new (opens wizard).

### Project creation wizard

Modal flow when the user adds a project. Steps:

1. **Pick a project root directory** (file picker).
2. **Name** + optional description.
3. **Choose a starting template** — bundled `DDD spec → plan → impl` / `Generic researcher → writer → reviewer` / `Empty` (start from canvas).
4. **Pre-flight checks** — Claude Code installed? `~/.claude/projects/` readable for transcript polling? Git available?
5. **Confirm** — wizard `mkdir`s the layout, copies template files, registers in `projects` table.

After creation, the app opens to the board view of that project.

## Usage tracking

### Two sources, three views

Per the deep-dive section, splitting transcript polling from stream-json tail:

- **Claude Code transcript watcher** — FSEvents/inotify on `~/.claude/projects/**/*.jsonl`. Each new line parsed; `message_id` deduplicated; `usage` extracted and inserted into `cc_usage_log`. Catches all CC usage on the machine including outside the app.
- **Our stream-json tail** — already happening in the worker loop. Inserts attributed rows into `worker_usage_log`.
- **Anthropic SDK tail** — for `api-runner` teams, the SDK response includes usage; appended to `worker_usage_log` (these calls are NOT in CC transcripts).

### Meter rendering

```
window_total = SUM(input + output) FROM (cc_usage_log UNION worker_usage_log WHERE runner='api') WHERE ts > now() - 5h
window_pct = window_total / configured_budget
```

Per-team breakdown for the tooltip queries `worker_usage_log` (we have team attribution there).

### Brake triggers

- **Manual** — user clicks brake state in topbar, or types `brake on` in the terminal.
- **Reactive** — any worker hits a rate-limit / quota error; brake set with reason; in-flight task released.
- **Auto** — `window_pct >= 0.95` (configurable). Brake set with reason `auto-meter`. Clears automatically once `window_pct < 0.85`.

When braked, the meter dims and shows `↻ <time until window resets>` in place of the burn rate.

## Git integration

- Workers operate inside `worktrees/<task_id>/` checkouts created from `target_repo`.
- **Never `git push`.** Tool-level deny in every team's scope. Tested in the wizard's pre-flight (`Bash(git push:*)` in deny list verified).
- **Never `git fetch` from inside a worker.** The user is responsible for keeping the target repo's main branch up to date. The app shows a topbar warning if `git -C <target_repo> rev-list HEAD..origin/main --count` (run by the app, not a worker) > 0 and asks the user to `git pull` manually.
- Worktree creation: `git -C <target_repo> worktree add <project>/worktrees/<task_id> -b agent-bus/<task_id> origin/main`. Branch name is opinionated; configurable in project settings.
- Worktree cleanup: when the user marks a task done in the UI (after pushing the branch externally), the app prompts `Remove worktree? Yes/No`. On yes: `git worktree remove`.
- Multi-repo: tasks have `target_repo` per-task; one project can have tasks targeting different repos.

## Theme + design tokens

See `DESIGN.md`. Summary:

- Default theme: **warm dark** (T1 — full mono). OS dark-mode preference drives initial value; user can toggle.
- Alternate: **warm light**. Same token vocabulary, inverted.
- Primary font: **Berkeley Mono** (paid; ship as woff2). Fallback to **JetBrains Mono**.
- Reading body inside artifact viewer: system-sans-serif via `ui-sans-serif` stack.
- One accent: ochre, reserved for needs-you + primary actions, ≤10% of surface.

## Distribution

- Repository: `agent-bus-app` (new repo to be created).
- Dependencies: Bun 1.x (frontend tooling), Rust toolchain (Tauri backend).
- Setup: `git clone && bun install && cargo tauri dev` (dev) / `cargo tauri build` (release).
- Releases: signed `.app` (macOS), `.msi` (Windows), `.deb` + `.AppImage` (Linux) via Tauri's bundler. Auto-update via Tauri's built-in updater pointing at GitHub releases.
- Bundle size target: <30MB.
- Configuration on first launch: `~/Library/Application Support/agent-bus-app/` (macOS) / `~/.config/agent-bus-app/` (Linux) / `%APPDATA%\agent-bus-app\` (Windows) for user prefs; project state lives in each project's `.agent-bus/`.

## v1 boundaries — what ships first

1. Tauri + React skeleton, dark + light themes, theme toggle.
2. Project wizard with the DDD pipeline template.
3. Board view with Kanban + card detail drawer (artifact + live log tabs).
4. Worker manager + `claude-cli` runner (API runner v1.1).
5. Pipeline engine — forward routing, revise with attempts cap, gates, escalations.
6. SQLite schema + task lifecycle.
7. Usage tracking from stream-json tail.
8. Inline comment system + revise compose flow.
9. Brake (manual + reactive); auto-meter v1.1.
10. God terminal v1 — basic inject/approve/brake commands; full tool surface v1.1.
11. Pipeline editor (read-only viewer in v1; editing v1.1).

## v1.1 — close-followups

- **API runner** (`anthropic-api`).
- **Pipeline editor write-mode** (drag-to-create, save YAML).
- **Auto-meter brake** based on CC transcript watcher.
- **List view** with filter pills.
- **Lineage tab** in card drawer.
- **Worktree cleanup prompts.**
- **Settings UI** for keychain-stored `ANTHROPIC_API_KEY`.

## v2 — later, scoped explicitly

- `sandbox-exec` / `bubblewrap` OS-level scope enforcement.
- Per-team brake (halt only expensive teams).
- Multiple projects open simultaneously in tabs.
- Cost dashboard with historical trends (week / month).
- Pipeline marketplace — share pipeline templates publicly.

## Open questions

- **Berkeley Mono is paid (\$75 single-user license).** Ship JetBrains Mono as the default? Or bundle Berkeley Mono and document the license? Lean: ship JetBrains Mono; Berkeley Mono is opt-in via Settings → General. Operator decision before release.

### Resolved during design (kept for audit trail)

- ✅ **Worktree base directory** — under the project root at `<project>/worktrees/<task_id>/`. See Git integration section.
- ✅ **Conversation persistence across app restarts** — yes; `conversations` SQLite table holds session history with a 24h-summarisation policy. See Data model → SQLite schema.
- ✅ **Skill / plugin discovery for CLI runner** — no runtime discovery; team prompts reference plugins by name (e.g. `/ddd-council critique`); the CLI's installed plugins set is implicit in what the prompt invokes. v1.1 may surface a checked list in the team-config UI.

## DDD model

A full strategic + tactical DDD model was produced via `/ddd-council map` in workshop register and ratified in-session. It lives in two documents that should be treated as the source of truth for context boundaries, relationships, aggregate roots, and invariants:

- **`DOMAIN.md`** — canonical roster: product, stack, seven bounded contexts, domain experts, ubiquitous language, detector config hint. Read first by any future `/ddd-council` invocation.
- **`docs/context-map.md`** — the strategic + tactical artifact: relationship table with DDD pattern names, dependency direction diagram, aggregate roots with invariants for each of the 7 contexts, end-to-end event flow diagram.

The seven contexts:

1. **Pipeline Authoring** *(supplier)* — graph definition
2. **Runtime** *(supplier)* — task lifecycle + worker pools (2 aggregates)
3. **Review** *(supplier)* — artifacts + comments + revisions
4. **Usage Telemetry** *(supplier)* — rolling window + brake
5. **Runners (ACL)** *(supplier)* — anti-corruption layer to Claude
6. **Workspace** *(supplier)* — shared kernel for path resolution
7. **Conversational Control** *(customer of all six)* — god terminal; thin context with one aggregate (Conversation)

Key relationship patterns:
- Workspace → all: **Shared Kernel** (paths)
- Pipeline Authoring ↔ Runtime: **Shared Kernel** with `schema_version` (hot-reload, save-validation prevents orphan in-flight tasks)
- Runtime → Review: **Customer-Supplier** (gated tasks)
- Review → Runtime: **Conformist** (verdicts in Runtime's vocabulary)
- Runtime → Runners: **Anti-Corruption Layer**
- Runners → Usage Telemetry: **Customer-Supplier** (usage events)
- Claude Code transcripts → Usage Telemetry: **Conformist**
- Conversational Control → each: **Customer** via published **Open Host Service** of app-tools

The Rust crate structure should mirror these context boundaries: one top-level module (or workspace crate) per context, each exposing an `api` sub-module as its OHS. The implementation plan downstream of this spec should organise tasks by context, building Workspace and Runtime first (per the focus declared in DOMAIN.md).

## Asset references

- **PRODUCT.md** — product positioning, anti-references, scene sentence
- **DESIGN.md** — full token vocabulary, components, anti-patterns
- **DOMAIN.md** — DDD canon (bounded contexts, experts, ubiquitous language)
- **docs/context-map.md** — DDD model (relationships, aggregates, invariants)
- **assets/designs/** — 8 screenshots of every iteration explored
- **assets/html/** — every HTML mockup, preserved for future reference
- **Predecessor system spec** — `~/splose/agent-bus-design.md` (file-queue + tmux era; the data-model insights transfer; the runtime model does not)
