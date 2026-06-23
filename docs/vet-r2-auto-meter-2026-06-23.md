---
id: vet-r2-auto-meter-2026-06-23
verb: vet
target: plans/2026-06-23-plan-r2-auto-meter.md
lens: strategic · vet · brief
date: 2026-06-23
verdict: CLEAN — no §E design smells; plan may proceed as written
---

# Vet — R2 Auto-meter Reactive Brake

**Scope of this gate:** DDD design soundness only (boundaries, ubiquitous language,
*refactor-before-add*). Decomposition / testability / sequencing are upstream
(`writing-plans`) and out of scope.

**Focus (operator-supplied):** the brake DECISION stays Usage Telemetry's, the brake
STATE stays Runtime's, wired only at the composition root (`app/src/lib.rs`) — the
Plan-5 pattern; no new cross-context edge; `usage_telemetry` stays acyclic.

## Evidence acquired

- `DOMAIN.md` — Usage Telemetry is a *supplier* that "decides when to set the brake";
  Runtime owns the `Brake` (system-wide flag). The two shared kernels are
  `agent_bus_core` + the Workspace path kernel.
- `src-tauri/usage_telemetry/Cargo.toml` — deps are **`agent_bus_core` + `workspace`
  only** (plus serde/sqlx/tokio/tauri). No `runtime` / `runners` edge. Acyclic.
- `src-tauri/usage_telemetry/src/{brake_policy.rs, snapshot.rs, api.rs}` — DECISION is
  pure (`decide`, `auto_brake_decision`); no reference to Runtime's `Brake`.
- `src-tauri/runtime/src/brake.rs` — `Brake` stores an opaque `Option<String>` reason;
  it does not know Telemetry's reason taxonomy (correct dependency direction).
- `src-tauri/app/src/lib.rs:752-780` — the composition root's 15s sweep is the *single*
  place that reads a `BrakeDecision` and calls `brake.set_on/set_off`. The plan does
  not change it.

## Findings

No findings. The plan introduces no §E design smell. Summary of the checks that
could have tripped:

- **§E1 feature-envy / cross-context reach — NOT present.** Every new line lands in the
  context that owns the concept. Task 1 (`auto_meter_enabled` on `UsageSnapshot`) and
  Task 2 (`set_auto_meter_inner` writing `usage_config`) operate on Telemetry's own
  config row. Task 3's dispatcher arm calls `set_auto_meter_inner` — it never touches
  `Brake`. Enabling/disabling is a Telemetry **config** write; only the pre-existing
  sweep applies a decision to Runtime's state.
- **§E2 misplaced concept — NOT present.** DECISION (`decide`/`auto_brake_decision`)
  stays pure in Telemetry; STATE stays in `runtime::brake::Brake`; the apply happens
  only at the root. This is the declared Plan-5 pattern, unchanged.
- **No new cross-context edge.** `usage_telemetry`'s dependency set is unchanged
  (verified against Cargo.toml). The root (`app`) already depends on both `runtime` and
  `usage_telemetry` — it is the customer/composition layer where wiring is permitted.
- **`usage_telemetry` stays acyclic — confirmed.** No path from `usage_telemetry` to
  `runtime`/`runners` exists or is added.
- **Refactor-before-add (the law vet enforces) — satisfied.** D-R2-1's new
  `usage_set_auto_meter` command is *not* an addition a refactor would obviate:
  budget (positive int) and auto-meter (bool) are different config fields with
  different validation; folding them into `usage_set_budget` would overload one
  command's semantics — the smell, not the cure. The dedicated command mirrors the
  existing single-responsibility OHS command shape.

## Friction surfaced (and resolved)

- **Engineer vs Architect — the string-typed brake reason.** The sweep computes
  `auto_on` by comparing `brake.state().reason == Some("auto-meter")`. Engineer flagged
  a stringly-typed invariant. Architect: the reason string *is* the cross-context
  contract — Runtime's `Brake` deliberately stores an opaque `Option<String>` so it
  needn't import Telemetry's reason taxonomy. Promoting it to a typed enum would push
  Telemetry vocabulary into Runtime, **inverting** the dependency direction. Converged:
  the `AUTO_METER_REASON` shared const (in `brake_policy`) is the correct seam.
  Pre-existing and correct; **not** a finding against this plan.

## Verdict

**CLEAN.** The plan keeps the DECISION in Usage Telemetry, the STATE in Runtime, the
wiring at the composition root, adds no cross-context edge, and keeps
`usage_telemetry` acyclic. No amendments required. Proceed to implement.
