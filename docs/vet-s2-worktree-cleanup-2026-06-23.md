---
id: vet-s2-worktree-cleanup-2026-06-23
verb: vet
target: plans/2026-06-23-plan-s2-worktree-cleanup.md
date: 2026-06-23
lens: strategic · vet · brief
verdict: SOUND WITH FIXES
---

# Vet — S2 Worktree-cleanup prompts

Pressure-test of `plans/2026-06-23-plan-s2-worktree-cleanup.md` against `DOMAIN.md`
and the affected code (`workspace/src/{paths.rs,api.rs,git_config.rs}`,
`runners/src/{claude_cli.rs,scope.rs}`, `runtime/src/pool.rs`,
`secrets` crate). DDD soundness only — decomposition/TDD/sequencing are upstream
(writing-plans) and not re-reviewed.

## Summary

The plan is **strategically sound**. Worktree management is correctly placed in the
**Workspace** context — Workspace is the declared owner of the project filesystem
layout (`DOMAIN.md` → Workspace: "Project root … contains … `worktrees/`";
`paths::project_subdirs()` lists `worktrees`). The git seam (`WorktreeGit` trait +
`GitCli` impl) faithfully mirrors the established ACL-seam pattern this repo already
uses three times: `SpawnFn` (`runners/src/claude_cli.rs`), the anthropic `SendFn`,
and the `KeychainStore` trait (`secrets` crate, S1). No git idiom (porcelain text,
`git worktree` argv, `std::process::Command`) crosses the OHS — `list_worktrees`
returns `Vec<WorktreeEntry>` and `remove_worktree` returns `Result<(), String>`. The
path-scoping guard (`worktree_under_root`) is pure and unit-tested independently of
git, satisfying the "never remove outside `worktrees/`" safety requirement.

Three low-severity findings, all language/honesty hygiene; none blocks the build.
All resolved by amending the plan (no redesign, no refactor-first escalation).

---

### F1 [low] off-language naming — "stale" is an unregistered, slightly-dishonest term

**What:** The `WorktreeEntry.stale: bool` field (plan Task 1) and the prose
"present *all* worktrees … as removable, labelling them 'not tied to an active
task'" (plan `## Decisions` → Staleness). `DOMAIN.md` has no "stale worktree" or
"cleanup candidate" concept; the Architect speaks Workspace's language and the term
isn't there. "Stale" also overstates: the rule is *not* "this worktree went stale"
but "this app tracks no task that owns it" — which today is **every** worktree under
`worktrees/`, because creation is absent.

**Cited:** plan Task 1 (`WorktreeEntry`), plan `## Decisions` (Staleness definition).

**Why it matters:** Off-language naming (§E off-language) puts a term in the code
that the ubiquitous language doesn't carry, and a slightly misleading one — a future
reader could infer the app distinguishes fresh-vs-stale worktrees when it does not.
The honest concept is "a cleanup candidate: a worktree under `worktrees/` tied to no
active task."

**Amendment (recommended):** Keep the field (the forward-compatible boolean is
useful when creation lands), but document it as **"cleanup candidate"** — the field
doc-comment already says so; tighten the prose to never call a worktree "stale" as if
the app aged it, and register the concept in `DOMAIN.md` → Workspace as **"Worktree
cleanup"** with the honest rule. Rename is optional and not worth a churn; the
doc-comment + DOMAIN registration carries it.

**Status:** resolved — plan `## Decisions` reworded to drop "stale" framing in favour
of "cleanup candidate / tied to no active task"; field doc-comment already honest;
DOMAIN.md Workspace section to gain a "Worktree cleanup" entry in the impeccable/merge
pass (plan Task 9 note added).

---

### F2 [low] unowned shared type? — `WorktreeEntry` crossing to the frontend

**What:** `WorktreeEntry` (Rust, `workspace/src/worktree.rs`) is serialized across
the OHS and re-declared in TS (`src/ipc/workspace.ts`). The question §E
unowned-shared-type asks: is this an accidental shared kernel that belongs in
`agent_bus_core`?

**Cited:** plan Task 1 (Rust DTO) + Task 6 (TS mirror).

**Why it matters:** A type two sides depend on, placed wrong, becomes an unowned
kernel. The vet must confirm ownership is single and intentional.

**Resolution (no change needed):** `WorktreeEntry` is **not** a cross-context kernel.
It is owned solely by Workspace and published through Workspace's *own* OHS to the
frontend — exactly the same shape as `Project` (`workspace/src/project.rs` →
`src/ipc/workspace.ts`, locked by `workspace/src/contract_tests.rs`). It belongs in
`agent_bus_core` only if a *second backend context* consumed it; none does. The
frontend is the OHS consumer, not a peer context. Placement in `workspace` is
correct.

**Amendment (recommended):** Add a serde wire-contract test for `WorktreeEntry`
mirroring `contract_tests.rs` (locks the key set/casing against the TS interface so a
drift fails `cargo test`). Cheap, matches the repo's established discipline.

**Status:** resolved — plan Task 1 gains a `WorktreeEntry` wire-contract test step
(key set { path, head, branch, stale } + value kinds), mirroring
`workspace/src/contract_tests.rs`.

---

### F3 [low] contradicts/clarifies DOMAIN.md — the creation-vs-cleanup honesty must land in DOMAIN.md

**What:** The plan is admirably honest in `## Decisions` that per-task worktree
*creation does not exist* (workers use `--add-dir` scopes, `runners/src/scope.rs`).
But that honesty lives only in the plan; `DOMAIN.md` → Workspace currently says only
that `worktrees/` exists and that "the worktree-commit path … lands in a later item"
(Git author identity entry) — a reader could still assume per-task worktrees are
created.

**Cited:** plan `## Decisions` (creation absent); `DOMAIN.md` Workspace section
(no cleanup concept, creation status implicit).

**Why it matters:** §E contradicts-DOMAIN (inverse): the plan's truth and DOMAIN.md's
silence diverge. The ubiquitous language should record that worktree **cleanup**
ships now while worktree **creation** remains a separate, currently-absent feature —
so the next item doesn't re-discover this or build on a false premise.

**Amendment (recommended):** Register a **"Worktree cleanup"** entry under
`DOMAIN.md` → Workspace stating: cleanup utility (`list_worktrees` /
`remove_worktree`, git behind the `WorktreeGit` seam, path-scoped to `worktrees/`)
ships in S2; per-task worktree *creation* is not implemented (workers run under
`--add-dir` scopes), so every discovered worktree is a cleanup candidate today.

**Status:** resolved — plan Task 9 gains a DOMAIN.md-registration step carrying the
creation-vs-cleanup honesty + the cleanup language.

---

## Verdict

**SOUND WITH FIXES.** Seams hold (Workspace owns it; git sealed behind a trait like
every other infra idiom in this repo); safety is enforced in pure, tested code;
naming is honest once F1/F3 land. All three findings are resolved by plan amendments —
no redesign, no refactor-first. Cleared to build.
