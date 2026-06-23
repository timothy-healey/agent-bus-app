---
id: 2026-06-23-vet-r5-pipeline-defaults
target: plans/2026-06-23-plan-r5-pipeline-defaults.md
date: 2026-06-23
verb: vet
mode: vet
lens: strategic · vet · brief
operator: tim.healey@splose.com
---

# Vet — R5 Pipeline-level runner/model defaults with team overrides

Pre-build DDD soundness gate on `plans/2026-06-23-plan-r5-pipeline-defaults.md`,
read against `DOMAIN.md`, `docs/context-map.md`, and the affected code
(`pipeline/src/{model,validate,parse,store,draft}.rs`, `runtime/src/pool.rs`,
`src/ipc/pipeline.ts`). Focus areas: (1) resolution belongs to Pipeline
Authoring; (2) the Authoring↔Runtime shared kernel keyed on `schema_version`;
(3) no resolution leak into Runtime; (4) naming vs DOMAIN.md.

**Verdict: sound.** The core seam is correct — resolution lives in Pipeline
Authoring (`resolve.rs`), the store resolves at load, Runtime consumes an
already-resolved effective `RunnerConfig` and never learns about defaults. No
schema-version bump (additive), so the shared kernel stays stable. Findings
below are language/documentation refinements and one design note; none block the
build. All recommended amendments applied.

## Findings

### F1 [low] off-language-naming — `effective_runner` / `TeamRunnerConfig` not in DOMAIN.md lexicon

**What.** The plan introduces `TeamRunnerConfig` (the authored partial override),
`PipelineDefaults`, and `Team::effective_runner()`. DOMAIN.md's Pipeline
Authoring ubiquitous language has *runner config*, *Effort*, *Runner kind* but no
term for "the resolved runner" or "the partial override". (Plan §Decisions DD2,
DD4; Tasks 1–2.)

**Why it matters.** *The language lives in the code* — a new public type/method
that isn't in DOMAIN.md drifts the lexicon. Low blast radius: the names are
descriptive and accurate, just unregistered.

**Friction.** Architect: `effective_runner` reads well and "effective X" is
standard for a resolved-with-defaults value; don't bikeshed. Domain expert
(Operator voice): the backlog already says `default_runner`/`default_model`, so
`PipelineDefaults` + `default_*` fields are *on* language; only the resolved-value
term is new. Converged: keep the names, register them.

**Amendment.** Adopt these as ubiquitous language and add a one-line note to
DOMAIN.md Pipeline Authoring: *"Effective runner config — a team's runner after
pipeline defaults are overlaid (R5); resolved by Pipeline Authoring at load so
Runtime always sees a fully-specified `RunnerConfig`."* Plan unchanged.

**Status:** resolved — DOMAIN.md note added (Task 0 below); names kept.

### F2 [low] contradicts-DOMAIN.md — "one runner config" vs optional `Team.runner`

**What.** DOMAIN.md → Pipeline Authoring: *"Team — a node in the graph with one
prompt, one scope, one runner config."* The plan makes `Team.runner`
`Option<TeamRunnerConfig>` (Plan DD2, Task 2), so a team may author *no* runner
config (inheriting the pipeline default).

**Why it matters.** Surface contradiction with the declared language. Real blast
radius is nil: the *effective* model preserves the invariant — post-resolution
every team has exactly one fully-specified runner config (enforced by
`validate`'s new `TeamHasNoRunner`, Task 4). The "one runner config" rule holds
on the resolved aggregate; only the *authored* form may omit it.

**Friction.** Engineer: the invariant the sentence protects ("a team always runs
with a defined runner") is *stronger* after R5, not weaker — validation now
rejects any team that can't resolve one. Architect: but the prose should say so,
else a future reader thinks `runner` is unconditionally required. Converged:
amend the language, not the design.

**Amendment.** Update DOMAIN.md Pipeline Authoring Team entry to: *"one prompt,
one scope, one runner config (a team may inherit the pipeline-level default and
override fields selectively — R5; the resolved team always has exactly one
fully-specified runner config)."* Plan unchanged.

**Status:** resolved — DOMAIN.md updated (Task 0 below).

### F3 [info] adds-where-a-refactor-fits — panicking `effective_runner()` accessor

**What.** `Team::effective_runner()` (Plan DD4, Task 2) `expect()`s on `None` /
missing fields, relying on the invariant that the store resolves + validates
before any consumer sees the pipeline.

**Why it matters.** A panicking accessor on a shared-kernel type is normally a
smell. Here it is the deliberate, cheapest way to hand Runtime an infallible
`RunnerConfig` without Runtime learning the `Option` (which would leak the
resolution concern across the seam). The alternative — `effective_runner() ->
Result` or `Option` — pushes a "can this be unresolved?" question into Runtime,
which is exactly the leak focus (3) forbids.

**Friction.** Engineer would normally reject a panic; Architect points out the
panic encodes the kernel invariant (*resolved before consumed*) at the seam,
keeping Runtime clean. Converged: accept the panic as an invariant guard, with a
clear message (the plan already specifies `"team runner not resolved (call
resolve::resolve_defaults first)"`).

**Amendment.** None required. Recommendation: ensure the panic message names the
resolver (the plan already does). Keep as-is.

**Status:** resolved — accepted as designed; no plan change.

### F4 [n/a] no-leak-into-Runtime — verified clean (focus 3)

**What.** Checked the resolution placement against the Authoring↔Runtime shared
kernel. `resolve_defaults` and the overlay precedence live entirely in
`pipeline::resolve` (Pipeline Authoring). `PipelineStore::load` resolves before
returning (Task 5). `runtime/src/pool.rs` reads only `team.effective_runner()`
(Task 7) — a method that returns a pre-resolved value and contains **no**
defaults/overlay logic.

**Why it matters.** This is the central concern. Runtime gains zero knowledge of
`PipelineDefaults`; the defaults concept never crosses into the Runtime context.
The shared kernel (`Team`/`RunnerConfig`, keyed on `schema_version`) is unchanged
in version — additive fields with `#[serde(default)]`, no `SCHEMA_VERSION` bump
(DD6) — so the kernel agreement between the two contexts is not renegotiated.

**Status:** resolved — no finding; seam confirmed correct.

## Applied amendments summary

- F1, F2 → DOMAIN.md language note (added as Task 0 in the plan; see below).
- F3 → accepted as designed, no change.
- F4 → no change; seam verified.

Plan body (Tasks 1–10) requires no structural revision. One documentation task
prepended.
