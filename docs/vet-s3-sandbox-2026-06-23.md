---
id: vet-s3-sandbox-2026-06-23
verb: vet
target: plans/2026-06-23-plan-s3-sandbox.md
date: 2026-06-23
lens: strategic · vet · brief
verdict: SOUND WITH FIXES
---

# Vet — S3 Sandbox hardening (`sandbox-exec`)

Reviewed: `plans/2026-06-23-plan-s3-sandbox.md` against `DOMAIN.md` (Runners ACL
context + ubiquitous language) and the affected code: `src-tauri/runners/src/{scope.rs,
command.rs, claude_cli.rs, output.rs, lib.rs}`, `src-tauri/runtime/src/pool.rs`,
`src-tauri/app/src/lib.rs`.

## Summary

The plan is DDD-sound on its load-bearing strategic claims. The Scope→SBPL mapping is a
**second projection of the same `Scope`** that `build_settings` already projects to
`settings.json` — it belongs in `runners/src/scope.rs` (refactor-before-add: no new crate,
no new context). The `sandbox-exec`/SBPL idiom stays **sealed inside the Runners ACL**:
the profile is generated and consumed entirely within the ACL, carried on the
ACL-owned `InvocationRequest` (already CLI-shaped — it holds `settings_path`/`add_dirs`),
and never crosses the `Runner` trait outward (Runtime flips only a `bool`). Flag default-OFF
keeps existing behavior byte-for-byte. The honesty discipline (experimental / Apple-deprecated
/ macOS-only / structural-only, no security-boundary overclaim) is correctly applied across
plan, doc-comments, and the planned backlog/report.

Four findings, none structural; all are language / seam-hygiene amendments applied in-plan.

## Findings

### F1 [medium] §E-language — the `Scope` ubiquitous-language entry is now too narrow

**What:** `DOMAIN.md` (Runners ACL, line 113) defines **Scope** as "the per-invocation
`settings.json` defining permission allow/deny". The plan adds a *second* artifact derived
from the same `Scope` (the SBPL profile). Leaving the definition pinned to `settings.json`
is a language-divergence smell — the term will mean more in code than in the glossary.

**Cited:** plan §Decisions D1, Task 1; `DOMAIN.md:113`.

**Why it matters:** One model per bounded context — the glossary term must match what the
code does, or the next reader believes Scope is settings-only and is surprised by the SBPL
projection. The fix is a glossary widening, not a code change (the concept is unchanged: a
per-invocation permission policy with two projections).

**Amendment:** Widen the `Scope` entry to: "the per-invocation **permission policy** derived
from a team's reads/writes/tools — projected to Claude's `settings.json` (allow/deny) and,
EXPERIMENTALLY (S3, opt-in, macOS-only), to a `sandbox-exec` SBPL profile. Both projections
live in `runners/src/scope.rs`." Add this to the backlog/DOMAIN-update step.

**Status:** resolved — DOMAIN.md `Scope` entry widened to name both projections (settings.json + experimental SBPL).

### F2 [low] §E-boundary — make the "SBPL never crosses `Runner`" seal explicit in the doc-comment, and the API-runner's ignore honest

**What:** D2 carries the profile on `InvocationRequest.sandbox_profile: Option<String>`.
That is correct (the struct is ACL-owned, CLI-shaped). But `InvocationRequest` is also the
input to `AnthropicApiRunner`, which has no subprocess to confine. Without an explicit note,
a reader could think the API runner silently drops a security control.

**Cited:** plan §Decisions D2; `output.rs:65` (`InvocationRequest`); `anthropic_api.rs`.

**Why it matters:** The ACL seal (the Runners context's defining invariant per `DOMAIN.md:24`)
is only trustworthy if the seam is documented where it's read. The field is CLI-idiom data
living on a struct shared with a non-CLI runner — that asymmetry must be stated, not implied.

**Amendment:** The plan's Task 3 doc-comment already states "AnthropicApiRunner ignores it
(no subprocess to confine)" — keep that, and add one line to the field doc clarifying that
`sandbox_profile` is **CLI-runner-only** and that Runtime never reads it (it only sets the
`PoolContext.sandbox` bool). No code shape change.

**Status:** resolved — field doc in plan Task 3 already carries the seal language; amendment folds the "CLI-runner-only / Runtime never reads it" line into it (applied during implementation).

### F3 [low] §E-language — name the experimental confinement consistently (`sandbox` bool reads as a verb/noun ambiguity)

**What:** The flag is `PoolContext.sandbox: bool` (D4, Task 5). `sandbox` alone is ambiguous
(a sandbox? to sandbox?) and doesn't carry the experimental/opt-in nature that the rest of
the language is careful to mark.

**Cited:** plan §Decisions D4, Task 5.

**Why it matters:** The language lives in the code; a bool that gates an experimental,
deprecated-API feature should read as exactly that at the call site (root sets it, pool
reads it).

**Amendment (recommended, auto-applied):** Keep the field name `sandbox` for brevity BUT
ensure the doc-comment (already in Task 5) leads with "EXPERIMENTAL (S3) · macOS-only" and
the root's literal carries the `TODO(s3)` + "OFF by default" comment (already planned). The
council judged a rename to `experimental_sandbox` as over-precise for a single private
PoolContext field whose doc-comment already states it — name stays `sandbox`, doc-comment is
the language carrier. (Recorded so the decision is visible, not silent.)

**Status:** resolved — name retained with the experimental doc-comment as the language carrier; decision recorded.

### F4 [low] §E-overclaim — keep the "not a proven boundary" caveat at every surface the term appears

**What:** The plan is honest in D6 and the doc-comments, but the council flags this as a
standing risk smell rather than a defect: a security feature's name ("sandbox") invites
readers to assume a proven boundary. The one place most likely to drift toward overclaim is
the **backlog entry** and the **report**.

**Cited:** plan §Decisions D6, Task 1/6 doc-comments, step 5 (backlog).

**Why it matters:** Honesty about verifiability is the operator's explicit constraint and a
domain-trust concern (the AI Engineer's lens: don't dress a structural change as a guarantee).

**Amendment:** Backlog S3 must be marked **DONE (with caveats) / structural-only** — NOT a
bare "done" — naming: profile-gen + argv-wrapping are tested; live confinement is unverified;
`sandbox-exec` is Apple-deprecated + macOS-only + opt-in default-OFF. The report carries the
same caveat in its one-liner and a prominent Caveats section.

**Status:** resolved — codified as the backlog-wording + report-wording requirement; applied at step 5 and in the final report.

## Verdict

**SOUND WITH FIXES.** No structural redesign and no pre-refactor required. The seams hold:
Scope owns both projections, the ACL stays sealed, and the default-OFF flag preserves
behavior. All four findings are language/doc-hygiene amendments folded into the existing
plan tasks. Proceed to implementation; apply F1 (DOMAIN/backlog glossary widening) and the
F2/F4 doc-comment + wording lines as the affected tasks land.
