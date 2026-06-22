# Agent Bus App — Plan 1: Foundation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a Tauri+React app that boots into a project-creation wizard, persists Project state in SQLite, applies the warm-dark/warm-light theme tokens, and exposes the `agent_bus_core` shared kernel. End state: launch `bun tauri dev`, see the app, click "New Project", create one, see it in a list with the active-project pill in the topbar.

**Architecture:** Tauri 2.x main process in Rust + WebKit/WebView2 webview. Cargo workspace with `agent_bus_core` (kernel crate, depends on nothing project-internal) and `workspace` (the Workspace context). React 18 + Vite frontend with TypeScript. Vanilla CSS using OKLCH-defined tokens from DESIGN.md. SQLite via `tauri-plugin-sql` with one migration for the `projects` and `conversations` tables.

**Tech Stack:** Rust (Tauri 2.x), React 18, TypeScript, Vite, Bun (package manager + script runner), tauri-plugin-sql, serde, vitest, @testing-library/react.

**Source spec:** `~/assistant/Efforts/agent-bus-app/2026-06-22-agent-bus-app-design.md`

**DDD anchor:** `~/assistant/Efforts/agent-bus-app/DOMAIN.md` (Plan 1 implements Workspace context + the `agent_bus_core` shared kernel)

---

## File structure

After Plan 1 completes, the repo looks like this:

```
agent-bus-app/                          # new repo, created in Task 1
├── .gitignore
├── README.md
├── package.json                        # bun workspace; React + Vite + TS deps
├── bun.lockb
├── vite.config.ts
├── tsconfig.json
├── index.html
├── src/                                # React frontend
│   ├── main.tsx
│   ├── App.tsx
│   ├── styles/
│   │   ├── tokens.css                  # OKLCH tokens (dark + light)
│   │   └── global.css                  # base styles, font loading
│   ├── components/
│   │   ├── Topbar.tsx
│   │   ├── Topbar.test.tsx
│   │   ├── ThemeToggle.tsx
│   │   ├── ThemeToggle.test.tsx
│   │   ├── ProjectWizard.tsx
│   │   ├── ProjectWizard.test.tsx
│   │   ├── ProjectList.tsx
│   │   ├── ProjectList.test.tsx
│   │   └── ViewSwitcher.tsx
│   ├── hooks/
│   │   ├── useTheme.ts
│   │   └── useProjects.ts
│   └── ipc/
│       └── workspace.ts                # typed wrappers for workspace Tauri commands
├── src-tauri/                          # Rust backend
│   ├── Cargo.toml                      # workspace
│   ├── tauri.conf.json
│   ├── build.rs
│   ├── icons/                          # default Tauri icons (generated)
│   ├── agent_bus_core/                 # shared kernel crate
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── ids.rs                  # TaskId, TeamId, PipelineId, ProjectId, ArtifactPath
│   │       ├── verdict.rs              # Verdict enum
│   │       ├── runner.rs               # RunnerKind, EffortMode
│   │       └── tool_protocol.rs        # ToolSpec, ToolCallRequest, ToolCallResult
│   ├── workspace/                      # Workspace context
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── project.rs              # Project struct + ProjectStore
│   │       └── api.rs                  # Tauri commands + tools()
│   └── app/                            # Tauri composition root
│       ├── Cargo.toml
│       ├── src/
│       │   ├── lib.rs
│       │   └── main.rs
│       └── migrations/
│           └── 001_initial.sql         # projects + conversations tables
└── tests/
    └── README.md                       # how to run rust + frontend tests
```

---

## Task 1: Create repo + Tauri scaffolding

**Files:**
- Create: new directory `~/projects/agent-bus-app/` (or your preferred location)
- Create: `~/projects/agent-bus-app/.gitignore`

- [ ] **Step 1: Verify Rust + Bun + Tauri prerequisites**

```bash
rustc --version           # expect 1.75+
cargo --version
bun --version             # expect 1.x
cargo install create-tauri-app --locked   # if not already
```

Expected: each command prints a version. If any fails, install per the official docs before continuing.

- [ ] **Step 2: Create the project via the Tauri scaffolder**

Run from a parent directory of your choosing (the example uses `~/projects/`):

```bash
mkdir -p ~/projects && cd ~/projects
cargo create-tauri-app
```

When prompted, choose:
- Project name: `agent-bus-app`
- Identifier: `com.tim-healey.agent-bus-app`
- Frontend language: **TypeScript / JavaScript (bun, npm, pnpm, yarn)**
- Package manager: **bun**
- UI template: **React**
- UI flavor: **TypeScript**

This produces `~/projects/agent-bus-app/` with a vanilla Tauri 2.x + React + Vite + TS layout.

- [ ] **Step 3: Verify the scaffold runs**

```bash
cd ~/projects/agent-bus-app
bun install
bun tauri dev
```

Expected: a Tauri window opens showing "Welcome to Tauri + React" with two button samples. Close the window.

- [ ] **Step 4: Init git, write `.gitignore`**

```bash
cd ~/projects/agent-bus-app
git init -q
```

Write `.gitignore` (append to whatever Tauri's scaffolder produced):

```gitignore
# Bun
node_modules/
bun.lockb.bak

# Tauri
src-tauri/target/
src-tauri/Cargo.lock

# Vite
dist/

# OS
.DS_Store
Thumbs.db

# IDE
.idea/
.vscode/*
!.vscode/extensions.json
!.vscode/launch.json

# App-local state (created at runtime)
*.db
*.db-journal
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: scaffold Tauri 2 + React 18 + TS + Vite app

Generated via cargo create-tauri-app with bun + React + TypeScript."
```

---

## Task 2: Cargo workspace layout

**Files:**
- Modify: `src-tauri/Cargo.toml` (convert to workspace)
- Create: `src-tauri/app/Cargo.toml`
- Move: existing `src-tauri/src/` → `src-tauri/app/src/`
- Move: existing `src-tauri/tauri.conf.json` stays at `src-tauri/tauri.conf.json` (referenced from app crate)
- Move: existing `src-tauri/build.rs` stays at root
- Modify: `src-tauri/tauri.conf.json` (build.frontendDist, build.beforeDevCommand stay; package name change reflected)

The scaffolder produces a single-crate layout. We need a workspace so `agent_bus_core` and `workspace` can be sibling crates.

- [ ] **Step 1: Move app sources into a subcrate**

```bash
cd ~/projects/agent-bus-app/src-tauri
mkdir -p app/src
git mv src app/
git mv Cargo.toml app/Cargo.toml
```

- [ ] **Step 2: Update `app/Cargo.toml` package name**

Edit `src-tauri/app/Cargo.toml` — change the `[package].name` from whatever the scaffolder used to `agent-bus-app`:

```toml
[package]
name = "agent-bus-app"
version = "0.1.0"
edition = "2021"
default-run = "agent-bus-app"
```

(Leave other fields the scaffolder set.)

- [ ] **Step 3: Create the workspace `Cargo.toml`**

Create `src-tauri/Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["app", "agent_bus_core", "workspace"]

[workspace.package]
version = "0.1.0"
edition = "2021"
authors = ["Tim Healey"]
license = "MIT OR Apache-2.0"

[workspace.dependencies]
tauri = { version = "2", features = [] }
tauri-build = { version = "2", features = [] }
tauri-plugin-sql = { version = "2", features = ["sqlite"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
sqlx = { version = "0.8", features = ["sqlite", "runtime-tokio"] }
tokio = { version = "1", features = ["full"] }
thiserror = "1"
uuid = { version = "1", features = ["v4", "serde"] }
```

- [ ] **Step 4: Verify the workspace builds**

```bash
cd ~/projects/agent-bus-app/src-tauri
cargo check --workspace 2>&1 | head -40
```

Expected: errors about missing `agent_bus_core` and `workspace` members (we haven't created them yet) OR the workspace builds the existing `app` crate. Either is fine for this step — we'll add the missing crates next.

If you see `error: failed to load manifest for workspace member`: that's the expected failure mode; we add those crates in Task 3.

- [ ] **Step 5: Commit**

```bash
cd ~/projects/agent-bus-app
git add -A
git commit -m "chore: convert src-tauri into a Cargo workspace

App crate moved to src-tauri/app. Workspace deps centralised in
src-tauri/Cargo.toml. agent_bus_core and workspace member stubs added
in following tasks."
```

---

## Task 3: `agent_bus_core` crate — ID newtypes

**Files:**
- Create: `src-tauri/agent_bus_core/Cargo.toml`
- Create: `src-tauri/agent_bus_core/src/lib.rs`
- Create: `src-tauri/agent_bus_core/src/ids.rs`

- [ ] **Step 1: Write the failing test (place it in `lib.rs` for now since the crate hasn't been created)**

Create `src-tauri/agent_bus_core/src/ids.rs` with **tests only**:

```rust
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TeamId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PipelineId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactPath(pub PathBuf);

impl fmt::Display for ProjectId { fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { self.0.fmt(f) } }
impl fmt::Display for TaskId    { fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { self.0.fmt(f) } }
impl fmt::Display for TeamId    { fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { self.0.fmt(f) } }
impl fmt::Display for PipelineId{ fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { self.0.fmt(f) } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_id_round_trips_through_serde() {
        let id = ProjectId("proj-abc".into());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"proj-abc\"");
        let back: ProjectId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn ids_are_not_interchangeable_at_the_type_level() {
        // This is enforced by the compiler, not at runtime — we just assert
        // the values are distinct types by constructing both.
        let p = ProjectId("x".into());
        let t = TaskId("x".into());
        assert_eq!(p.0, t.0);
        // The line below MUST NOT compile if uncommented (different types).
        // assert_eq!(p, t);
    }

    #[test]
    fn artifact_path_serialises_as_string() {
        let ap = ArtifactPath(PathBuf::from("/tmp/foo.md"));
        let json = serde_json::to_string(&ap).unwrap();
        assert_eq!(json, "\"/tmp/foo.md\"");
    }
}
```

- [ ] **Step 2: Create the crate Cargo.toml**

Create `src-tauri/agent_bus_core/Cargo.toml`:

```toml
[package]
name = "agent_bus_core"
version.workspace = true
edition.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
```

- [ ] **Step 3: Create the crate `lib.rs`**

Create `src-tauri/agent_bus_core/src/lib.rs`:

```rust
//! agent_bus_core — shared kernel for cross-context primitives.
//!
//! Per DOMAIN.md and the design spec, this crate holds the ID newtypes,
//! cross-context enums, and OHS protocol types. It depends on nothing
//! project-internal.

pub mod ids;

pub use ids::*;
```

- [ ] **Step 4: Run the tests**

```bash
cd ~/projects/agent-bus-app/src-tauri
cargo test -p agent_bus_core --lib ids
```

Expected:

```
running 3 tests
test ids::tests::artifact_path_serialises_as_string ... ok
test ids::tests::ids_are_not_interchangeable_at_the_type_level ... ok
test ids::tests::project_id_round_trips_through_serde ... ok

test result: ok. 3 passed; 0 failed
```

- [ ] **Step 5: Commit**

```bash
cd ~/projects/agent-bus-app
git add -A
git commit -m "feat(core): add ID newtypes (Project/Task/Team/Pipeline/ArtifactPath)

Each ID is a String/PathBuf newtype with serde + Display impls.
Distinct types prevent accidental cross-use at the type level."
```

---

## Task 4: `agent_bus_core` — `Verdict` and runner enums

**Files:**
- Create: `src-tauri/agent_bus_core/src/verdict.rs`
- Create: `src-tauri/agent_bus_core/src/runner.rs`
- Modify: `src-tauri/agent_bus_core/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/agent_bus_core/src/verdict.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Approve,
    Revise,
    Reject,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_serialises_as_lowercase_string() {
        assert_eq!(serde_json::to_string(&Verdict::Approve).unwrap(), "\"approve\"");
        assert_eq!(serde_json::to_string(&Verdict::Revise).unwrap(),  "\"revise\"");
        assert_eq!(serde_json::to_string(&Verdict::Reject).unwrap(),  "\"reject\"");
    }

    #[test]
    fn verdict_round_trips() {
        for v in [Verdict::Approve, Verdict::Revise, Verdict::Reject] {
            let s = serde_json::to_string(&v).unwrap();
            let back: Verdict = serde_json::from_str(&s).unwrap();
            assert_eq!(v, back);
        }
    }
}
```

Create `src-tauri/agent_bus_core/src/runner.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerKind {
    ClaudeCli,
    AnthropicApi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum EffortMode {
    Off,
    Standard,
    ExtendedLow,
    ExtendedHigh,
    Custom { budget_tokens: u32 },
}

impl EffortMode {
    /// Resolve to the actual thinking-token budget Claude should use.
    pub fn budget_tokens(self) -> u32 {
        match self {
            EffortMode::Off => 0,
            EffortMode::Standard => 1024,
            EffortMode::ExtendedLow => 8192,
            EffortMode::ExtendedHigh => 32000,
            EffortMode::Custom { budget_tokens } => budget_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_kind_serialises_as_kebab_case() {
        assert_eq!(serde_json::to_string(&RunnerKind::ClaudeCli).unwrap(), "\"claude-cli\"");
        assert_eq!(serde_json::to_string(&RunnerKind::AnthropicApi).unwrap(), "\"anthropic-api\"");
    }

    #[test]
    fn effort_mode_preset_budgets() {
        assert_eq!(EffortMode::Off.budget_tokens(), 0);
        assert_eq!(EffortMode::Standard.budget_tokens(), 1024);
        assert_eq!(EffortMode::ExtendedLow.budget_tokens(), 8192);
        assert_eq!(EffortMode::ExtendedHigh.budget_tokens(), 32000);
        assert_eq!(EffortMode::Custom { budget_tokens: 16000 }.budget_tokens(), 16000);
    }

    #[test]
    fn effort_mode_serialises_with_internal_tag() {
        let e = EffortMode::Standard;
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"mode\":\"standard\""), "got: {s}");
    }
}
```

- [ ] **Step 2: Wire the new modules into `lib.rs`**

Edit `src-tauri/agent_bus_core/src/lib.rs`:

```rust
//! agent_bus_core — shared kernel for cross-context primitives.

pub mod ids;
pub mod verdict;
pub mod runner;

pub use ids::*;
pub use verdict::*;
pub use runner::*;
```

- [ ] **Step 3: Run tests**

```bash
cd ~/projects/agent-bus-app/src-tauri
cargo test -p agent_bus_core
```

Expected: all 8 tests pass (3 from ids + 2 from verdict + 3 from runner).

- [ ] **Step 4: Commit**

```bash
cd ~/projects/agent-bus-app
git add -A
git commit -m "feat(core): add Verdict + RunnerKind + EffortMode enums

Verdict serialises as lowercase string; RunnerKind as kebab-case;
EffortMode has an internal 'mode' tag plus a budget_tokens() resolver
that maps presets to thinking-token budgets per DOMAIN.md."
```

---

## Task 5: `agent_bus_core` — tool protocol

**Files:**
- Create: `src-tauri/agent_bus_core/src/tool_protocol.rs`
- Modify: `src-tauri/agent_bus_core/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/agent_bus_core/src/tool_protocol.rs`:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Describes an app-tool a supplier context publishes as part of its OHS.
/// The Conversational Control context consumes the union of all suppliers'
/// ToolSpec values to build the god terminal's tool catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Tool name as Claude sees it (e.g. "inject_topic", "approve_gate").
    pub name: String,
    /// Human-readable description (shown to Claude).
    pub description: String,
    /// JSON Schema for the tool's arguments.
    pub input_schema: Value,
    /// The supplier context that owns this tool.
    pub supplier_context: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub tool_name: String,
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolCallResult {
    Ok { result: Value },
    Err { error: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_spec_round_trips() {
        let spec = ToolSpec {
            name: "inject_topic".into(),
            description: "Inject a new topic into the research team's inbox.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "topic": { "type": "string" } },
                "required": ["topic"]
            }),
            supplier_context: "runtime".into(),
        };
        let s = serde_json::to_string(&spec).unwrap();
        let back: ToolSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(spec, back);
    }

    #[test]
    fn tool_call_result_serialises_with_status_tag() {
        let ok = ToolCallResult::Ok { result: json!({"task_id": "T-042"}) };
        let err = ToolCallResult::Err { error: "rate-limited".into() };

        let ok_json = serde_json::to_string(&ok).unwrap();
        let err_json = serde_json::to_string(&err).unwrap();

        assert!(ok_json.contains("\"status\":\"ok\""));
        assert!(err_json.contains("\"status\":\"err\""));
    }
}
```

- [ ] **Step 2: Wire into `lib.rs`**

Edit `src-tauri/agent_bus_core/src/lib.rs`:

```rust
//! agent_bus_core — shared kernel for cross-context primitives.

pub mod ids;
pub mod verdict;
pub mod runner;
pub mod tool_protocol;

pub use ids::*;
pub use verdict::*;
pub use runner::*;
pub use tool_protocol::*;
```

- [ ] **Step 3: Run tests**

```bash
cd ~/projects/agent-bus-app/src-tauri
cargo test -p agent_bus_core
```

Expected: 10 tests pass (3 ids + 2 verdict + 3 runner + 2 tool_protocol).

- [ ] **Step 4: Commit**

```bash
cd ~/projects/agent-bus-app
git add -A
git commit -m "feat(core): add OHS tool protocol types

ToolSpec describes an app-tool exposed by a supplier context as part of
its Open Host Service; ToolCallRequest/Result are the dispatch envelope.
The Conversational Control context (Plan 6) consumes the union of all
suppliers' ToolSpec values to build the terminal's tool catalog."
```

---

## Task 6: SQLite + initial migration

**Files:**
- Modify: `src-tauri/app/Cargo.toml`
- Create: `src-tauri/app/migrations/001_initial.sql`
- Modify: `src-tauri/app/src/lib.rs` (or `main.rs`, depending on scaffold)
- Modify: `src-tauri/tauri.conf.json` (add sql plugin to allowlist if Tauri 2 requires)

- [ ] **Step 1: Add `tauri-plugin-sql` to the app crate**

Edit `src-tauri/app/Cargo.toml`. The `[dependencies]` block becomes:

```toml
[dependencies]
serde.workspace = true
serde_json.workspace = true
tauri.workspace = true
tauri-plugin-sql.workspace = true
tokio.workspace = true
agent_bus_core = { path = "../agent_bus_core" }

[build-dependencies]
tauri-build.workspace = true
```

Also add the frontend dependency. Edit `~/projects/agent-bus-app/package.json`:

```json
{
  "dependencies": {
    "@tauri-apps/api": "^2",
    "@tauri-apps/plugin-sql": "^2",
    "react": "^18.3.0",
    "react-dom": "^18.3.0"
  }
}
```

Then:

```bash
cd ~/projects/agent-bus-app
bun install
```

- [ ] **Step 2: Write the initial migration**

Create `src-tauri/app/migrations/001_initial.sql`:

```sql
-- 001_initial.sql — Plan 1 schema (Workspace + Conversation tables only;
-- other tables land in their respective context's plan).

CREATE TABLE IF NOT EXISTS projects (
  id                TEXT PRIMARY KEY,
  name              TEXT NOT NULL,
  root_path         TEXT NOT NULL,
  active_pipeline   TEXT,
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS conversations (
  id                          TEXT PRIMARY KEY,
  project_id                  TEXT NOT NULL,
  started_at                  INTEGER NOT NULL,
  last_message_at             INTEGER NOT NULL,
  history_json                TEXT NOT NULL,
  summary_of_prior_sessions   TEXT,
  FOREIGN KEY(project_id) REFERENCES projects(id)
);

CREATE INDEX IF NOT EXISTS idx_conversations_project ON conversations(project_id);
```

- [ ] **Step 3: Register the SQL plugin in the Rust app**

Inspect what your scaffolder produced — Tauri 2's scaffolder may have a `lib.rs` exporting `run()` plus a thin `main.rs`. Edit `src-tauri/app/src/lib.rs`:

```rust
use tauri_plugin_sql::{Migration, MigrationKind};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let migrations = vec![
        Migration {
            version: 1,
            description: "initial — projects + conversations",
            sql: include_str!("../migrations/001_initial.sql"),
            kind: MigrationKind::Up,
        },
    ];

    tauri::Builder::default()
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations("sqlite:agent_bus.db", migrations)
                .build(),
        )
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

(`main.rs` from the scaffold typically just calls `agent_bus_app_lib::run()`. Leave it.)

- [ ] **Step 4: Run the app to verify the migration applies**

```bash
cd ~/projects/agent-bus-app
bun tauri dev
```

The app should launch without crashing. Close it. Then verify the database was created and has the tables:

```bash
find ~/Library/Application\ Support -name "agent_bus.db" 2>/dev/null | head -1
# Or on Linux:
# find ~/.local/share -name "agent_bus.db" 2>/dev/null | head -1
```

The path is `~/Library/Application Support/com.tim-healey.agent-bus-app/agent_bus.db` on macOS. Inspect tables:

```bash
sqlite3 ~/Library/Application\ Support/com.tim-healey.agent-bus-app/agent_bus.db ".schema"
```

Expected: both `projects` and `conversations` tables defined.

- [ ] **Step 5: Commit**

```bash
cd ~/projects/agent-bus-app
git add -A
git commit -m "feat(app): wire tauri-plugin-sql with initial migration

Migration 001 creates the projects + conversations tables. SQLite db
lives at the app's platform-appropriate data dir as agent_bus.db."
```

---

## Task 7: Workspace crate — `Project` struct

**Files:**
- Create: `src-tauri/workspace/Cargo.toml`
- Create: `src-tauri/workspace/src/lib.rs`
- Create: `src-tauri/workspace/src/project.rs`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/workspace/src/project.rs`:

```rust
use agent_bus_core::{PipelineId, ProjectId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub root_path: PathBuf,
    pub active_pipeline: Option<PipelineId>,
    pub created_at: i64,    // unix epoch seconds
    pub updated_at: i64,
}

impl Project {
    /// Create a new in-memory Project. Caller is responsible for persistence
    /// (see ProjectStore in api.rs).
    pub fn new(name: String, root_path: PathBuf, now_unix: i64) -> Self {
        Self {
            id: ProjectId(format!("proj-{}", uuid::Uuid::new_v4())),
            name,
            root_path,
            active_pipeline: None,
            created_at: now_unix,
            updated_at: now_unix,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_new_generates_proj_prefixed_id() {
        let p = Project::new("Test".into(), "/tmp/test".into(), 1_700_000_000);
        assert!(p.id.0.starts_with("proj-"), "id was {}", p.id.0);
        assert_eq!(p.name, "Test");
        assert_eq!(p.root_path, PathBuf::from("/tmp/test"));
        assert_eq!(p.active_pipeline, None);
        assert_eq!(p.created_at, 1_700_000_000);
        assert_eq!(p.updated_at, 1_700_000_000);
    }

    #[test]
    fn project_round_trips_through_serde() {
        let p = Project::new("X".into(), "/p".into(), 1);
        let s = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
```

- [ ] **Step 2: Create the crate Cargo.toml + lib.rs**

Create `src-tauri/workspace/Cargo.toml`:

```toml
[package]
name = "workspace"
version.workspace = true
edition.workspace = true

[dependencies]
agent_bus_core = { path = "../agent_bus_core" }
serde.workspace = true
serde_json.workspace = true
uuid.workspace = true
thiserror.workspace = true
```

Create `src-tauri/workspace/src/lib.rs`:

```rust
//! workspace — the Workspace context. Owns Project state + path resolution
//! (path resolution lands in a later task; this file initially exposes only
//! the Project type and store).

pub mod project;

pub use project::*;
```

- [ ] **Step 3: Run tests**

```bash
cd ~/projects/agent-bus-app/src-tauri
cargo test -p workspace
```

Expected: 2 tests pass.

- [ ] **Step 4: Commit**

```bash
cd ~/projects/agent-bus-app
git add -A
git commit -m "feat(workspace): add Project struct with constructor + serde

Project carries ProjectId, name, root_path, active_pipeline (Option),
and created/updated timestamps. Project::new generates a 'proj-<uuid>'
ID. Tests cover construction defaults + serde round-trip."
```

---

## Task 8: Workspace crate — `ProjectStore` over SQLite

**Files:**
- Create: `src-tauri/workspace/src/store.rs`
- Modify: `src-tauri/workspace/src/lib.rs`
- Modify: `src-tauri/workspace/Cargo.toml`

- [ ] **Step 1: Add the `sqlx` dependency**

Edit `src-tauri/workspace/Cargo.toml` — add to `[dependencies]`:

```toml
sqlx = { workspace = true }
tokio = { workspace = true }
```

- [ ] **Step 2: Write the failing tests + store skeleton**

Create `src-tauri/workspace/src/store.rs`:

```rust
use crate::project::Project;
use agent_bus_core::{PipelineId, ProjectId};
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectStoreError {
    #[error("project not found: {0}")]
    NotFound(ProjectId),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub struct ProjectStore {
    pool: SqlitePool,
}

impl ProjectStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, project: &Project) -> Result<(), ProjectStoreError> {
        sqlx::query(
            "INSERT INTO projects (id, name, root_path, active_pipeline, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&project.id.0)
        .bind(&project.name)
        .bind(project.root_path.to_string_lossy().to_string())
        .bind(project.active_pipeline.as_ref().map(|p| &p.0))
        .bind(project.created_at)
        .bind(project.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<Project>, ProjectStoreError> {
        let rows = sqlx::query_as::<_, (String, String, String, Option<String>, i64, i64)>(
            "SELECT id, name, root_path, active_pipeline, created_at, updated_at
             FROM projects ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(id, name, root_path, active, created, updated)| Project {
            id: ProjectId(id),
            name,
            root_path: root_path.into(),
            active_pipeline: active.map(PipelineId),
            created_at: created,
            updated_at: updated,
        }).collect())
    }

    pub async fn get(&self, id: &ProjectId) -> Result<Project, ProjectStoreError> {
        let row = sqlx::query_as::<_, (String, String, String, Option<String>, i64, i64)>(
            "SELECT id, name, root_path, active_pipeline, created_at, updated_at
             FROM projects WHERE id = ?",
        )
        .bind(&id.0)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((id, name, root_path, active, created, updated)) => Ok(Project {
                id: ProjectId(id),
                name,
                root_path: root_path.into(),
                active_pipeline: active.map(PipelineId),
                created_at: created,
                updated_at: updated,
            }),
            None => Err(ProjectStoreError::NotFound(id.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_and_list_round_trip() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);

        let p = Project::new("Demo".into(), "/tmp/demo".into(), 1_700_000_000);
        store.insert(&p).await.unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], p);
    }

    #[tokio::test]
    async fn get_missing_returns_not_found() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);

        let result = store.get(&ProjectId("nope".into())).await;
        assert!(matches!(result, Err(ProjectStoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn list_orders_by_created_at_desc() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);

        let mut earlier = Project::new("First".into(), "/p1".into(), 100);
        let later = Project::new("Second".into(), "/p2".into(), 200);
        earlier.updated_at = 100;
        store.insert(&earlier).await.unwrap();
        store.insert(&later).await.unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(listed[0].name, "Second");
        assert_eq!(listed[1].name, "First");
    }
}
```

- [ ] **Step 3: Wire `store` module into `lib.rs`**

Edit `src-tauri/workspace/src/lib.rs`:

```rust
//! workspace — the Workspace context.

pub mod project;
pub mod store;

pub use project::*;
pub use store::*;
```

- [ ] **Step 4: Run tests**

```bash
cd ~/projects/agent-bus-app/src-tauri
cargo test -p workspace
```

Expected: 5 tests pass (2 from `project` + 3 from `store`).

- [ ] **Step 5: Commit**

```bash
cd ~/projects/agent-bus-app
git add -A
git commit -m "feat(workspace): add ProjectStore over SQLite

ProjectStore wraps a SqlitePool with insert/list/get methods. Tests
exercise round-trip + not-found + ordering against an in-memory
SQLite instance running the same migration file the app uses."
```

---

## Task 9: Workspace crate — Tauri commands

**Files:**
- Create: `src-tauri/workspace/src/api.rs`
- Modify: `src-tauri/workspace/src/lib.rs`
- Modify: `src-tauri/workspace/Cargo.toml`

- [ ] **Step 1: Add the `tauri` dependency to workspace crate**

Edit `src-tauri/workspace/Cargo.toml` — add to `[dependencies]`:

```toml
tauri = { workspace = true }
```

- [ ] **Step 2: Write the API module + tests**

Create `src-tauri/workspace/src/api.rs`:

```rust
//! Tauri commands published by the Workspace context — the context's
//! Open Host Service surface.

use crate::project::Project;
use crate::store::{ProjectStore, ProjectStoreError};
use agent_bus_core::{ProjectId, ToolSpec};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Shared state held by Tauri's state manager.
pub struct WorkspaceState {
    pub store: Arc<ProjectStore>,
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

#[tauri::command]
pub async fn workspace_create_project(
    state: tauri::State<'_, WorkspaceState>,
    name: String,
    root_path: String,
) -> Result<Project, String> {
    let project = Project::new(name, PathBuf::from(root_path), now_unix());
    state.store.insert(&project).await.map_err(|e| e.to_string())?;
    Ok(project)
}

#[tauri::command]
pub async fn workspace_list_projects(
    state: tauri::State<'_, WorkspaceState>,
) -> Result<Vec<Project>, String> {
    state.store.list().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workspace_get_project(
    state: tauri::State<'_, WorkspaceState>,
    id: String,
) -> Result<Project, String> {
    state.store.get(&ProjectId(id)).await.map_err(|e| match e {
        ProjectStoreError::NotFound(_) => "not_found".to_string(),
        other => other.to_string(),
    })
}

/// OHS contract: the union of these is what Conversational Control will
/// expose to the god terminal in Plan 6.
pub fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "workspace_create_project".into(),
            description: "Create a new project in the workspace.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "root_path": { "type": "string" }
                },
                "required": ["name", "root_path"]
            }),
            supplier_context: "workspace".into(),
        },
        ToolSpec {
            name: "workspace_list_projects".into(),
            description: "List all known projects, newest first.".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
            supplier_context: "workspace".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_publishes_workspace_named_tools() {
        let t = tools();
        assert!(t.iter().all(|s| s.supplier_context == "workspace"));
        assert!(t.iter().any(|s| s.name == "workspace_create_project"));
        assert!(t.iter().any(|s| s.name == "workspace_list_projects"));
    }
}
```

- [ ] **Step 3: Wire `api` into `lib.rs`**

Edit `src-tauri/workspace/src/lib.rs`:

```rust
//! workspace — the Workspace context.

pub mod project;
pub mod store;
pub mod api;

pub use project::*;
pub use store::*;
```

(Note: we don't re-export `api::*` at the crate root — Tauri commands are referenced explicitly via `workspace::api::workspace_create_project` etc. from the composition root.)

- [ ] **Step 4: Run tests**

```bash
cd ~/projects/agent-bus-app/src-tauri
cargo test -p workspace
```

Expected: 6 tests pass (2 project + 3 store + 1 api).

- [ ] **Step 5: Commit**

```bash
cd ~/projects/agent-bus-app
git add -A
git commit -m "feat(workspace): add Tauri commands + tools() OHS surface

workspace_create_project, workspace_list_projects, workspace_get_project
are the Workspace context's commands. tools() returns the ToolSpec list
the Conversational Control context will consume in Plan 6."
```

---

## Task 10: Register workspace commands in the app composition root

**Files:**
- Modify: `src-tauri/app/Cargo.toml`
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Add `workspace` as an app dependency**

Edit `src-tauri/app/Cargo.toml` — add to `[dependencies]`:

```toml
workspace = { path = "../workspace" }
sqlx = { workspace = true }
```

- [ ] **Step 2: Register commands + initialise the ProjectStore**

Edit `src-tauri/app/src/lib.rs`:

```rust
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_sql::{Migration, MigrationKind};
use workspace::{api::WorkspaceState, store::ProjectStore};

const DB_URL: &str = "sqlite:agent_bus.db";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let migrations = vec![
        Migration {
            version: 1,
            description: "initial — projects + conversations",
            sql: include_str!("../migrations/001_initial.sql"),
            kind: MigrationKind::Up,
        },
    ];

    tauri::Builder::default()
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations(DB_URL, migrations)
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                // Resolve the DB path the plugin uses and open our own pool
                // against it. We can't share the plugin's pool directly, so
                // we open a sibling connection — sqlite handles concurrency.
                let data_dir = handle
                    .path()
                    .app_data_dir()
                    .expect("no app data dir");
                std::fs::create_dir_all(&data_dir).ok();
                let db_path = data_dir.join("agent_bus.db");

                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .connect(&format!("sqlite://{}", db_path.display()))
                    .await
                    .expect("could not open project store pool");

                let store = Arc::new(ProjectStore::new(pool));
                handle.manage(WorkspaceState { store });
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workspace::api::workspace_create_project,
            workspace::api::workspace_list_projects,
            workspace::api::workspace_get_project,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 3: Run the app to verify it boots**

```bash
cd ~/projects/agent-bus-app
bun tauri dev
```

Expected: app window opens (still showing the Tauri default page). No panic in the console.

Close the window.

- [ ] **Step 4: Verify the commands compile**

```bash
cd ~/projects/agent-bus-app/src-tauri
cargo check --workspace
```

Expected: clean check, no errors.

- [ ] **Step 5: Commit**

```bash
cd ~/projects/agent-bus-app
git add -A
git commit -m "feat(app): register workspace::api commands at composition root

Composition root opens a SqlitePool against the same DB the sql plugin
manages (sibling connection) and installs WorkspaceState. The three
workspace commands are now invokable from the frontend."
```

---

## Task 11: Frontend — token CSS + theme toggle

**Files:**
- Create: `src/styles/tokens.css`
- Create: `src/styles/global.css`
- Modify: `src/main.tsx`
- Create: `src/hooks/useTheme.ts`
- Create: `src/components/ThemeToggle.tsx`
- Create: `src/components/ThemeToggle.test.tsx`

- [ ] **Step 1: Write the tokens**

Create `src/styles/tokens.css`:

```css
/* OKLCH design tokens from DESIGN.md. Two themes via data-theme. */

:root[data-theme="dark"] {
  --bg:         oklch(11%  0.005 60);
  --bg-2:       oklch(9%   0.004 60);
  --surface:    oklch(15%  0.005 60);
  --surface-2: oklch(18%  0.006 60);
  --surface-3: oklch(22%  0.008 60);
  --border:    oklch(24%  0.008 60);
  --border-2:  oklch(32%  0.010 60);
  --text:      oklch(92%  0.008 70);
  --text-2:    oklch(72%  0.008 70);
  --text-3:    oklch(55%  0.008 70);
  --text-4:    oklch(42%  0.008 70);
  --accent:    oklch(74%  0.13  55);
  --accent-2:  oklch(20%  0.040 55);
  --accent-3:  oklch(28%  0.060 55);
  --accent-bd: oklch(48%  0.090 55);
  --running:   oklch(72%  0.08  145);
  --running-2: oklch(22%  0.030 145);
  --revise:    oklch(70%  0.10  305);
  --revise-2:  oklch(20%  0.035 305);
  --danger:    oklch(70%  0.12  25);
  --danger-2:  oklch(22%  0.04  25);
  --shadow-card: 0 1px 2px oklch(0% 0.01 70 / 0.20),
                 0 4px 12px oklch(0% 0.01 70 / 0.30);
}

:root[data-theme="light"] {
  --bg:        oklch(98%  0.005 70);
  --bg-2:      oklch(96%  0.006 70);
  --surface:   oklch(99.5% 0.003 70);
  --surface-2: oklch(96%  0.006 70);
  --surface-3: oklch(93%  0.008 70);
  --border:    oklch(89%  0.008 70);
  --border-2:  oklch(82%  0.008 70);
  --text:      oklch(22%  0.012 70);
  --text-2:    oklch(45%  0.010 70);
  --text-3:    oklch(60%  0.008 70);
  --text-4:    oklch(72%  0.006 70);
  --accent:    oklch(58%  0.135 55);
  --accent-2:  oklch(96%  0.030 70);
  --accent-3:  oklch(92%  0.050 60);
  --accent-bd: oklch(80%  0.060 60);
  --running:   oklch(48%  0.060 145);
  --running-2: oklch(92%  0.020 145);
  --revise:    oklch(48%  0.090 305);
  --revise-2:  oklch(96%  0.020 305);
  --danger:    oklch(48%  0.13  25);
  --danger-2:  oklch(96%  0.02  25);
  --shadow-card: 0 1px 2px oklch(20% 0.01 70 / 0.04),
                 0 4px 12px oklch(20% 0.01 70 / 0.03);
}

/* spacing scale */
:root {
  --sp-1: 4px;   --sp-2: 8px;   --sp-3: 12px;  --sp-4: 14px;
  --sp-5: 16px;  --sp-6: 18px;  --sp-7: 20px;  --sp-8: 24px;
  --sp-9: 28px;  --sp-10: 32px; --sp-12: 48px; --sp-14: 64px;
  --r-xs: 2px;   --r-sm: 3px;   --r-md: 4px;   --r-lg: 6px;
  --r-pill: 999px;
}
```

Create `src/styles/global.css`:

```css
@import "./tokens.css";

* { box-sizing: border-box; }
html, body, #root { height: 100%; margin: 0; }

body {
  font-family: 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 13px;
  line-height: 1.5;
  background: var(--bg);
  color: var(--text);
  -webkit-font-smoothing: antialiased;
  font-feature-settings: 'ss01', 'ss02';
}

code, .mono {
  font-family: 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace;
  font-variant-numeric: tabular-nums;
}
```

- [ ] **Step 2: Import global stylesheet at the entry**

Edit `src/main.tsx` — add the import at the top:

```typescript
import "./styles/global.css";
```

(Leave the rest of the file untouched.)

- [ ] **Step 3: Write the failing test for `ThemeToggle`**

Install vitest + testing-library if not already present:

```bash
cd ~/projects/agent-bus-app
bun add -d vitest @testing-library/react @testing-library/jest-dom @vitest/ui jsdom @types/react @types/react-dom
```

Edit `vite.config.ts` — add the test config block. (If your scaffolder produced a Vite config without a test block, add it.)

```typescript
/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
  },
  clearScreen: false,
  server: { port: 1420, strictPort: true },
});
```

Create `src/test-setup.ts`:

```typescript
import "@testing-library/jest-dom/vitest";
```

Create `src/components/ThemeToggle.test.tsx`:

```typescript
import { describe, expect, it, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ThemeToggle } from "./ThemeToggle";

describe("ThemeToggle", () => {
  beforeEach(() => {
    document.documentElement.removeAttribute("data-theme");
    localStorage.clear();
  });

  it("renders a button labelled with the current theme target", () => {
    document.documentElement.setAttribute("data-theme", "dark");
    render(<ThemeToggle />);
    expect(screen.getByRole("button")).toHaveAccessibleName(/light mode/i);
  });

  it("toggles data-theme on click", () => {
    document.documentElement.setAttribute("data-theme", "dark");
    render(<ThemeToggle />);
    fireEvent.click(screen.getByRole("button"));
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("persists the chosen theme in localStorage", () => {
    document.documentElement.setAttribute("data-theme", "dark");
    render(<ThemeToggle />);
    fireEvent.click(screen.getByRole("button"));
    expect(localStorage.getItem("agent-bus-app-theme")).toBe("light");
  });
});
```

- [ ] **Step 4: Run the test (it fails because the component doesn't exist)**

```bash
cd ~/projects/agent-bus-app
bun vitest run src/components/ThemeToggle.test.tsx
```

Expected: FAIL — "Cannot find module './ThemeToggle'".

- [ ] **Step 5: Implement the hook + component**

Create `src/hooks/useTheme.ts`:

```typescript
import { useEffect, useState } from "react";

export type Theme = "dark" | "light";
const STORAGE_KEY = "agent-bus-app-theme";

export function useTheme(): [Theme, (next: Theme) => void] {
  const [theme, setTheme] = useState<Theme>(() => {
    if (typeof window === "undefined") return "dark";
    const stored = localStorage.getItem(STORAGE_KEY) as Theme | null;
    if (stored === "dark" || stored === "light") return stored;
    const prefersDark = window.matchMedia?.("(prefers-color-scheme: dark)").matches;
    return prefersDark ? "dark" : "light";
  });

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem(STORAGE_KEY, theme);
  }, [theme]);

  return [theme, setTheme];
}
```

Create `src/components/ThemeToggle.tsx`:

```typescript
import { useTheme } from "../hooks/useTheme";

export function ThemeToggle() {
  const [theme, setTheme] = useTheme();
  const next: "dark" | "light" = theme === "dark" ? "light" : "dark";
  const label = `${next === "light" ? "Light" : "Dark"} mode`;

  return (
    <button
      onClick={() => setTheme(next)}
      aria-label={label}
      style={{
        background: "transparent",
        border: "1px solid var(--border)",
        color: "var(--text-2)",
        padding: "4px 10px",
        borderRadius: "var(--r-sm)",
        fontFamily: "inherit",
        fontSize: "11px",
        cursor: "pointer",
      }}
    >
      {label}
    </button>
  );
}
```

- [ ] **Step 6: Run tests + commit**

```bash
cd ~/projects/agent-bus-app
bun vitest run src/components/ThemeToggle.test.tsx
```

Expected: 3 tests pass.

```bash
git add -A
git commit -m "feat(ui): tokens.css + ThemeToggle + useTheme hook

Tokens cover dark + light (warm-tinted neutrals + ochre accent) per
DESIGN.md. ThemeToggle reads/writes data-theme on <html> and persists
to localStorage with prefers-color-scheme as the initial default."
```

---

## Task 12: Frontend — typed Tauri IPC for workspace commands

**Files:**
- Create: `src/ipc/workspace.ts`
- Create: `src/ipc/workspace.test.ts`

- [ ] **Step 1: Write the failing test**

Create `src/ipc/workspace.test.ts`:

```typescript
import { describe, expect, it, vi, beforeEach } from "vitest";
import { createProject, listProjects } from "./workspace";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe("workspace ipc", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("createProject calls workspace_create_project with name + root_path", async () => {
    const project = {
      id: "proj-abc",
      name: "Demo",
      root_path: "/tmp/demo",
      active_pipeline: null,
      created_at: 1,
      updated_at: 1,
    };
    invokeMock.mockResolvedValueOnce(project);

    const result = await createProject({ name: "Demo", root_path: "/tmp/demo" });

    expect(invokeMock).toHaveBeenCalledWith("workspace_create_project", {
      name: "Demo",
      root_path: "/tmp/demo",
    });
    expect(result).toEqual(project);
  });

  it("listProjects calls workspace_list_projects and returns array", async () => {
    invokeMock.mockResolvedValueOnce([]);
    const result = await listProjects();
    expect(invokeMock).toHaveBeenCalledWith("workspace_list_projects");
    expect(result).toEqual([]);
  });
});
```

- [ ] **Step 2: Run, see failure**

```bash
cd ~/projects/agent-bus-app
bun vitest run src/ipc/workspace.test.ts
```

Expected: FAIL — "Cannot find module './workspace'".

- [ ] **Step 3: Implement the IPC wrapper**

Create `src/ipc/workspace.ts`:

```typescript
import { invoke } from "@tauri-apps/api/core";

export interface Project {
  id: string;
  name: string;
  root_path: string;
  active_pipeline: string | null;
  created_at: number;
  updated_at: number;
}

export interface CreateProjectArgs {
  name: string;
  root_path: string;
}

export async function createProject(args: CreateProjectArgs): Promise<Project> {
  return await invoke<Project>("workspace_create_project", args);
}

export async function listProjects(): Promise<Project[]> {
  return await invoke<Project[]>("workspace_list_projects");
}

export async function getProject(id: string): Promise<Project> {
  return await invoke<Project>("workspace_get_project", { id });
}
```

- [ ] **Step 4: Run, see pass**

```bash
bun vitest run src/ipc/workspace.test.ts
```

Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(ui): typed workspace IPC wrapper

src/ipc/workspace.ts exposes createProject/listProjects/getProject as
async functions over Tauri's invoke. Mirrors the Project shape returned
by the Rust workspace::api commands."
```

---

## Task 13: Frontend — ProjectList component

**Files:**
- Create: `src/components/ProjectList.tsx`
- Create: `src/components/ProjectList.test.tsx`
- Create: `src/hooks/useProjects.ts`

- [ ] **Step 1: Write the failing test**

Create `src/components/ProjectList.test.tsx`:

```typescript
import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { ProjectList } from "./ProjectList";

vi.mock("../ipc/workspace", () => ({
  listProjects: vi.fn().mockResolvedValue([
    {
      id: "proj-1",
      name: "Splose DDD",
      root_path: "/Users/tim/splose",
      active_pipeline: null,
      created_at: 1700000000,
      updated_at: 1700000000,
    },
    {
      id: "proj-2",
      name: "Side project",
      root_path: "/Users/tim/side",
      active_pipeline: "ddd-spec-plan-impl",
      created_at: 1700000100,
      updated_at: 1700000100,
    },
  ]),
}));

describe("ProjectList", () => {
  it("renders the projects returned by listProjects", async () => {
    render(<ProjectList />);
    await waitFor(() => {
      expect(screen.getByText("Splose DDD")).toBeInTheDocument();
      expect(screen.getByText("Side project")).toBeInTheDocument();
    });
  });

  it("shows the root_path for each project", async () => {
    render(<ProjectList />);
    await waitFor(() => {
      expect(screen.getByText("/Users/tim/splose")).toBeInTheDocument();
    });
  });

  it("shows empty state when no projects exist", async () => {
    const { listProjects } = await import("../ipc/workspace");
    (listProjects as ReturnType<typeof vi.fn>).mockResolvedValueOnce([]);

    render(<ProjectList />);
    await waitFor(() => {
      expect(screen.getByText(/no projects yet/i)).toBeInTheDocument();
    });
  });
});
```

- [ ] **Step 2: Run, see failure**

```bash
bun vitest run src/components/ProjectList.test.tsx
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement the hook and component**

Create `src/hooks/useProjects.ts`:

```typescript
import { useEffect, useState } from "react";
import { listProjects, type Project } from "../ipc/workspace";

export function useProjects(): { projects: Project[]; loading: boolean; reload: () => void } {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    listProjects()
      .then((list) => { if (!cancelled) setProjects(list); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [tick]);

  return { projects, loading, reload: () => setTick((t) => t + 1) };
}
```

Create `src/components/ProjectList.tsx`:

```typescript
import { useProjects } from "../hooks/useProjects";

export function ProjectList() {
  const { projects, loading } = useProjects();

  if (loading) {
    return <div style={{ padding: "var(--sp-8)", color: "var(--text-3)" }}>Loading projects…</div>;
  }

  if (projects.length === 0) {
    return (
      <div style={{ padding: "var(--sp-8)", color: "var(--text-3)", textAlign: "center" }}>
        No projects yet. Use <strong style={{ color: "var(--text)" }}>New Project</strong> in the topbar.
      </div>
    );
  }

  return (
    <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
      {projects.map((p) => (
        <li
          key={p.id}
          style={{
            padding: "var(--sp-4) var(--sp-7)",
            borderBottom: "1px solid var(--border)",
          }}
        >
          <div style={{ color: "var(--text)", fontWeight: 500 }}>{p.name}</div>
          <div style={{ color: "var(--text-3)", fontSize: "11px" }}>{p.root_path}</div>
        </li>
      ))}
    </ul>
  );
}
```

- [ ] **Step 4: Run, see pass**

```bash
bun vitest run src/components/ProjectList.test.tsx
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(ui): ProjectList + useProjects hook

ProjectList renders a vertical list pulling from listProjects() via
the useProjects hook. Empty state nudges the user to the topbar."
```

---

## Task 14: Frontend — ProjectWizard component

**Files:**
- Create: `src/components/ProjectWizard.tsx`
- Create: `src/components/ProjectWizard.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `src/components/ProjectWizard.test.tsx`:

```typescript
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ProjectWizard } from "./ProjectWizard";

vi.mock("../ipc/workspace", () => ({
  createProject: vi.fn(),
}));

import { createProject } from "../ipc/workspace";

describe("ProjectWizard", () => {
  beforeEach(() => {
    (createProject as ReturnType<typeof vi.fn>).mockReset();
  });

  it("renders when open=true", () => {
    render(<ProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByLabelText(/project name/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/root path/i)).toBeInTheDocument();
  });

  it("does not render when open=false", () => {
    render(<ProjectWizard open={false} onClose={() => {}} onCreated={() => {}} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("calls createProject + onCreated on submit", async () => {
    const created = {
      id: "proj-x",
      name: "New",
      root_path: "/tmp/n",
      active_pipeline: null,
      created_at: 1,
      updated_at: 1,
    };
    (createProject as ReturnType<typeof vi.fn>).mockResolvedValueOnce(created);

    const onCreated = vi.fn();
    render(<ProjectWizard open={true} onClose={() => {}} onCreated={onCreated} />);

    fireEvent.change(screen.getByLabelText(/project name/i), { target: { value: "New" } });
    fireEvent.change(screen.getByLabelText(/root path/i), { target: { value: "/tmp/n" } });
    fireEvent.click(screen.getByRole("button", { name: /create project/i }));

    await waitFor(() => {
      expect(createProject).toHaveBeenCalledWith({ name: "New", root_path: "/tmp/n" });
      expect(onCreated).toHaveBeenCalledWith(created);
    });
  });

  it("disables submit when fields empty", () => {
    render(<ProjectWizard open={true} onClose={() => {}} onCreated={() => {}} />);
    const button = screen.getByRole("button", { name: /create project/i });
    expect(button).toBeDisabled();
  });
});
```

- [ ] **Step 2: Run, see failure**

```bash
bun vitest run src/components/ProjectWizard.test.tsx
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement the wizard**

Create `src/components/ProjectWizard.tsx`:

```typescript
import { FormEvent, useState } from "react";
import { createProject, type Project } from "../ipc/workspace";

export interface ProjectWizardProps {
  open: boolean;
  onClose: () => void;
  onCreated: (p: Project) => void;
}

export function ProjectWizard({ open, onClose, onCreated }: ProjectWizardProps) {
  const [name, setName] = useState("");
  const [rootPath, setRootPath] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!open) return null;

  const canSubmit = name.trim().length > 0 && rootPath.trim().length > 0 && !submitting;

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    try {
      const project = await createProject({ name: name.trim(), root_path: rootPath.trim() });
      onCreated(project);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.5)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 100,
      }}
      onClick={onClose}
    >
      <form
        onSubmit={handleSubmit}
        onClick={(e) => e.stopPropagation()}
        style={{
          background: "var(--surface)",
          border: "1px solid var(--border)",
          borderRadius: "var(--r-md)",
          padding: "var(--sp-8)",
          minWidth: 420,
          boxShadow: "var(--shadow-card)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: 14, color: "var(--text)" }}>New project</h2>
        <p style={{ color: "var(--text-3)", fontSize: 11, marginTop: "var(--sp-1)", marginBottom: "var(--sp-7)" }}>
          Pick a name and a root directory. The directory will hold pipelines, artifacts, and worktrees.
        </p>

        <label style={{ display: "block", fontSize: 11, color: "var(--text-3)", marginBottom: 4 }}>
          Project name
        </label>
        <input
          aria-label="Project name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          style={inputStyle}
        />

        <label style={{ display: "block", fontSize: 11, color: "var(--text-3)", marginBottom: 4, marginTop: "var(--sp-4)" }}>
          Root path
        </label>
        <input
          aria-label="Root path"
          value={rootPath}
          onChange={(e) => setRootPath(e.target.value)}
          placeholder="/Users/you/projects/example"
          style={inputStyle}
        />

        {error && (
          <div style={{ color: "var(--danger)", fontSize: 11, marginTop: "var(--sp-3)" }}>{error}</div>
        )}

        <div style={{ display: "flex", justifyContent: "flex-end", gap: "var(--sp-2)", marginTop: "var(--sp-7)" }}>
          <button type="button" onClick={onClose} style={ghostButton}>Cancel</button>
          <button type="submit" disabled={!canSubmit} style={canSubmit ? primaryButton : disabledButton}>
            Create project
          </button>
        </div>
      </form>
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  width: "100%",
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  color: "var(--text)",
  padding: "var(--sp-2) var(--sp-3)",
  fontFamily: "inherit",
  fontSize: 12,
  borderRadius: "var(--r-sm)",
};

const primaryButton: React.CSSProperties = {
  background: "var(--accent)",
  border: "1px solid var(--accent)",
  color: "oklch(15% 0.04 55)",
  padding: "var(--sp-2) var(--sp-4)",
  fontFamily: "inherit",
  fontSize: 12,
  borderRadius: "var(--r-sm)",
  cursor: "pointer",
  fontWeight: 500,
};

const ghostButton: React.CSSProperties = {
  background: "transparent",
  border: "1px solid var(--border)",
  color: "var(--text-2)",
  padding: "var(--sp-2) var(--sp-4)",
  fontFamily: "inherit",
  fontSize: 12,
  borderRadius: "var(--r-sm)",
  cursor: "pointer",
};

const disabledButton: React.CSSProperties = {
  ...primaryButton,
  opacity: 0.5,
  cursor: "not-allowed",
};
```

- [ ] **Step 4: Run, see pass**

```bash
bun vitest run src/components/ProjectWizard.test.tsx
```

Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(ui): ProjectWizard modal with name + root_path fields

Wizard renders as a dialog when open=true; submit calls createProject
and invokes onCreated. Submit disabled while fields are empty or
submission is in flight. Errors surface inline below the form."
```

---

## Task 15: Frontend — Topbar component

**Files:**
- Create: `src/components/Topbar.tsx`
- Create: `src/components/Topbar.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `src/components/Topbar.test.tsx`:

```typescript
import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Topbar } from "./Topbar";

describe("Topbar", () => {
  it("shows the brand mark", () => {
    render(<Topbar activeProject={null} onNewProject={() => {}} />);
    expect(screen.getByText(/agent bus/i)).toBeInTheDocument();
  });

  it("shows 'No project' pill when activeProject is null", () => {
    render(<Topbar activeProject={null} onNewProject={() => {}} />);
    expect(screen.getByText(/no project/i)).toBeInTheDocument();
  });

  it("shows the project name when activeProject is set", () => {
    render(
      <Topbar
        activeProject={{
          id: "proj-1",
          name: "Splose",
          root_path: "/x",
          active_pipeline: null,
          created_at: 1,
          updated_at: 1,
        }}
        onNewProject={() => {}}
      />
    );
    expect(screen.getByText("Splose")).toBeInTheDocument();
  });

  it("calls onNewProject when 'New project' is clicked", () => {
    const handler = vi.fn();
    render(<Topbar activeProject={null} onNewProject={handler} />);
    fireEvent.click(screen.getByRole("button", { name: /new project/i }));
    expect(handler).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run, see failure**

```bash
bun vitest run src/components/Topbar.test.tsx
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement Topbar**

Create `src/components/Topbar.tsx`:

```typescript
import { type Project } from "../ipc/workspace";
import { ThemeToggle } from "./ThemeToggle";

export interface TopbarProps {
  activeProject: Project | null;
  onNewProject: () => void;
}

export function Topbar({ activeProject, onNewProject }: TopbarProps) {
  return (
    <header
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-5)",
        padding: "var(--sp-3) var(--sp-8)",
        borderBottom: "1px solid var(--border)",
        background: "var(--surface)",
      }}
    >
      <div style={{ fontWeight: 600, fontSize: 14, color: "var(--text)" }}>
        <span style={{ color: "var(--accent)" }}>●</span>&nbsp;agent bus
      </div>

      <div
        className="mono"
        style={{
          color: "var(--text-2)",
          fontSize: 12,
          padding: "2px 10px",
          border: "1px solid var(--border)",
          borderRadius: "var(--r-pill)",
          background: "var(--bg)",
        }}
      >
        {activeProject ? activeProject.name : "No project"}
      </div>

      <div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: "var(--sp-2)" }}>
        <button
          onClick={onNewProject}
          style={{
            background: "var(--accent)",
            border: "1px solid var(--accent)",
            color: "oklch(15% 0.04 55)",
            padding: "4px 10px",
            fontFamily: "inherit",
            fontSize: 11,
            borderRadius: "var(--r-sm)",
            cursor: "pointer",
            fontWeight: 500,
          }}
        >
          New project
        </button>
        <ThemeToggle />
      </div>
    </header>
  );
}
```

- [ ] **Step 4: Run, see pass**

```bash
bun vitest run src/components/Topbar.test.tsx
```

Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(ui): Topbar component with brand + project pill + actions

Topbar shows ● agent bus, the active-project pill (or 'No project'),
right-aligned 'New project' + ThemeToggle. Calls onNewProject prop on
click."
```

---

## Task 16: Wire it all together in `App.tsx`

**Files:**
- Modify: `src/App.tsx`

- [ ] **Step 1: Replace the scaffolder's App with the real shell**

Edit `src/App.tsx`:

```typescript
import { useCallback, useState } from "react";
import { Topbar } from "./components/Topbar";
import { ProjectList } from "./components/ProjectList";
import { ProjectWizard } from "./components/ProjectWizard";
import { useProjects } from "./hooks/useProjects";
import { type Project } from "./ipc/workspace";

export default function App() {
  const { projects, reload } = useProjects();
  const [wizardOpen, setWizardOpen] = useState(false);

  // For Plan 1, the "active project" is just the newest one in the list.
  // Plan 2 introduces explicit active-project selection.
  const activeProject: Project | null = projects[0] ?? null;

  const onCreated = useCallback((_p: Project) => {
    setWizardOpen(false);
    reload();
  }, [reload]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <Topbar activeProject={activeProject} onNewProject={() => setWizardOpen(true)} />
      <main style={{ flex: 1, overflow: "auto" }}>
        <ProjectList />
      </main>
      <ProjectWizard
        open={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onCreated={onCreated}
      />
    </div>
  );
}
```

- [ ] **Step 2: Run the dev server and verify end-to-end**

```bash
cd ~/projects/agent-bus-app
bun tauri dev
```

Verify manually:

1. App opens; topbar visible; "No project" pill shown.
2. Empty state in the main area says "No projects yet."
3. Click "New project". Wizard opens.
4. Type a name ("Test") and a root path (any existing directory, e.g. `/tmp`). Click "Create project".
5. Wizard closes. The project appears in the list. The topbar pill now shows "Test".
6. Click the theme toggle. The theme flips light↔dark; the colour palette switches.
7. Close the app and re-launch with `bun tauri dev`. The project is still there (persisted in SQLite).

If any of these fail, inspect the console (developer tools menu from Tauri's window menu) and the Rust log output in the terminal.

- [ ] **Step 3: Run the full test suite**

```bash
cd ~/projects/agent-bus-app
cargo test --manifest-path src-tauri/Cargo.toml --workspace
bun vitest run
```

Expected: all tests pass — 16 Rust tests + 13 vitest tests = 29 total.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(app): wire Topbar + ProjectList + ProjectWizard in App

App holds the project list as state via useProjects; New Project opens
the wizard; on create, reload + close. Active project derived from the
list's newest entry until Plan 2 introduces explicit selection."
```

---

## Task 17: README, plan documentation, and final verification

**Files:**
- Modify: `README.md`
- Create: `tests/README.md`

- [ ] **Step 1: Write the project README**

Replace `README.md` contents:

```markdown
# agent-bus-app

A local Tauri + React app for orchestrating multi-team Claude Code agent
pipelines. Built from the spec at `~/assistant/Efforts/agent-bus-app/`.

## Status

**Plan 1 complete** — foundation only. Project creation + persistence + theme
toggle work. No pipeline runtime yet (Plans 2–6).

## Run

```bash
bun install
bun tauri dev
```

## Test

```bash
# Rust unit tests (all crates)
cargo test --manifest-path src-tauri/Cargo.toml --workspace

# Frontend tests (vitest)
bun vitest run
```

## Structure

The Rust workspace mirrors the DDD bounded contexts from the spec:

```
src-tauri/
├── agent_bus_core/        shared kernel — ID newtypes, enums, OHS protocol
├── workspace/             Workspace context — projects + path resolution
└── app/                   composition root — Tauri runtime + migrations
```

Frontend:

```
src/
├── components/            React components (one per file, colocated test)
├── hooks/                 useTheme, useProjects
├── ipc/                   typed wrappers over Tauri commands
└── styles/                tokens.css + global.css
```

See `~/assistant/Efforts/agent-bus-app/` for the full spec, DOMAIN.md,
context-map, and DESIGN tokens.
```

- [ ] **Step 2: Write the testing notes**

Create `tests/README.md`:

```markdown
# Testing — Plan 1

## Rust (workspace)

```bash
cargo test --manifest-path src-tauri/Cargo.toml --workspace
```

By crate:

```bash
cargo test -p agent_bus_core
cargo test -p workspace
```

## Frontend (vitest)

```bash
bun vitest          # watch mode
bun vitest run      # single pass
bun vitest --ui     # browser UI
```

## End-to-end (manual for Plan 1)

Plan 1 does not include Tauri E2E (WebDriver) tests — deferred to a later
plan. Verify the wizard + persistence manually per Task 16 step 2.

## Coverage targets

- `agent_bus_core`: 100% (the kernel; no excuse)
- `workspace`: 90%+ on the store and api modules
- React components: every prop variant + every interaction handler
```

- [ ] **Step 3: Run everything one more time**

```bash
cd ~/projects/agent-bus-app
cargo test --manifest-path src-tauri/Cargo.toml --workspace
bun vitest run
bun tauri dev   # then manually verify per Task 16 step 2
```

Expected: tests pass; app behaves as described.

- [ ] **Step 4: Commit and tag**

```bash
git add -A
git commit -m "docs: add README + tests/README

Plan 1 complete. Foundation is in place. Plan 2 (Pipeline Authoring +
viewer) is the next milestone."
git tag -a plan-1-foundation -m "Plan 1: Foundation complete"
```

---

## Self-review

### Spec coverage

Walking the spec sections against Plan 1's tasks:

| Spec requirement | Plan 1 task |
|---|---|
| Tauri 2.x + React 18 + TS + Vite | Task 1, 2 |
| Cargo workspace layout | Task 2 |
| `agent_bus_core` shared kernel — IDs | Task 3 |
| `agent_bus_core` — `Verdict`, `RunnerKind`, `EffortMode` | Task 4 |
| `agent_bus_core` — `ToolSpec`, `ToolCallRequest/Result` | Task 5 |
| SQLite via `tauri-plugin-sql` + initial migration | Task 6 |
| `projects` SQLite table | Task 6 |
| `conversations` SQLite table | Task 6 |
| Workspace context — Project type + store | Task 7, 8 |
| Workspace Tauri commands as `api::*` per F1 amendment | Task 9, 10 |
| Workspace `tools()` OHS surface per F6 amendment | Task 9 |
| Theme tokens (warm dark + warm light) | Task 11 |
| Theme toggle persisting to localStorage | Task 11 |
| Typed Tauri IPC wrapper | Task 12 |
| Project list UI | Task 13 |
| Project creation wizard | Task 14 |
| Topbar (brand + project pill + new-project button + theme) | Task 15 |
| Integration in `App.tsx` | Task 16 |
| README + testing docs | Task 17 |

**Out of scope for Plan 1** (covered in later plans):
- Pipeline Authoring context, YAML parsing, pipeline viewer → Plan 2
- View switcher tabs (board/list/pipeline/settings) — only the topbar lands here; switcher in Plan 4
- All Runtime, Review, Telemetry, Conversational Control, Runners work → Plans 3–6
- Path resolution kernel for Workspace (the `${project}`, `${target_repo}` variables) → Plan 2 (when Pipeline Authoring needs them)

### Placeholder scan

Searched for: `TBD`, `TODO`, `implement later`, `handle edge cases`, `similar to`. None found.

### Type consistency

`Project` shape matches between Rust (`workspace::Project`) and TS (`src/ipc/workspace.ts`). Field names align: `id` / `name` / `root_path` / `active_pipeline` / `created_at` / `updated_at`. Verified.

The `tools()` function in Workspace's `api.rs` returns `Vec<ToolSpec>` with all fields populated per the `agent_bus_core::ToolSpec` shape — name, description, input_schema (as `serde_json::Value`), supplier_context.

---

## Roadmap — what Plan 1 unblocks

| Plan | Scope | Depends on |
|---|---|---|
| **2** | Pipeline Authoring — YAML schema, validation, bundled DDD template, pipeline viewer | Plan 1 |
| **3** | Runtime + Runners — task lifecycle, claude-cli runner, worker pool, scope enforcement | Plans 1, 2 |
| **4** | Board UI + Review — Kanban, card drawer, inline comments, gate actions | Plans 1, 2, 3 |
| **5** | Usage Telemetry — transcript watcher, meter, brake | Plans 1, 3 |
| **6** | Conversational Control — god terminal, tool catalog wiring | All previous |
| **7** | Polish — list view, lineage tab, animations, settings, hardening | All previous |

Each subsequent plan ships its own milestone; this plan's `plan-1-foundation` tag is the reference point.
