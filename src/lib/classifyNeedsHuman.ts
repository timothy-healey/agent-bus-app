import type { InvocationRow } from "../ipc/runtime";
import type { Pipeline } from "../ipc/pipeline";

/// How a `needs_human` card is framed (spec H — Classification). A pure display
/// concern derived from the audit trail + the pipeline; never a persisted field.
///
/// - **failure** — a failure escalation: the latest invocation errored, was
///   rejected at a gate, or exhausted its revises. The card offers recovery
///   actions (Retry / Approve & advance / Abandon).
/// - **handoff** — an intentional hand-off: the work reached the pipeline's
///   terminal escalation node via a clean approve. The card offers Accept /
///   Send back ("ready for you").
export type NeedsHumanKind = "failure" | "handoff";

/// Classify a `needs_human` card from its latest audit outcome + the pipeline.
///
/// Decision rule (spec):
///   latest outcome `error:*` | `verdict:reject` | exhausted-revise ⇒ failure
///   a clean latest `verdict:approve` into the terminal escalation     ⇒ handoff
///
/// `invocations` is the task's audit trail NEWEST-FIRST (as `list_invocations`
/// returns it), so the latest outcome is `invocations[0]`. An empty trail has no
/// recorded failure to recover from and no clean approve to accept — we default
/// to **failure** so the operator still gets the recovery affordances rather than
/// a dead-end (the conservative choice; a hand-off is only ever the result of a
/// recorded clean approve).
///
/// "Exhausted-revise": the latest outcome is a `verdict:revise` that nonetheless
/// escalated — i.e. a reviewer asked for another pass but the item could not
/// revise further (attempts cap). It surfaces here as a revise outcome on a card
/// that is `needs_human`, so a latest `verdict:revise` is treated as a failure.
export function classifyNeedsHuman(
  invocations: InvocationRow[],
  pipeline: Pipeline | null,
): NeedsHumanKind {
  const latest = invocations[0];
  if (!latest) return "failure";

  const outcome = latest.outcome;
  // Any operational error, a reject, or an exhausted revise is a failure.
  if (outcome.startsWith("error:")) return "failure";
  if (outcome === "verdict:reject") return "failure";
  if (outcome === "verdict:revise") return "failure";

  // A clean approve is a hand-off ONLY when the pipeline declares a terminal
  // escalation node (the "hand off to human" sink). Without one, a stray approve
  // that still landed needs_human is treated as a failure (no terminal to accept
  // into).
  if (outcome === "verdict:approve") {
    const hasEscalation = (pipeline?.escalations.length ?? 0) > 0;
    return hasEscalation ? "handoff" : "failure";
  }

  // Unknown / in-flight ("") outcome — no recorded clean approve, so failure.
  return "failure";
}
