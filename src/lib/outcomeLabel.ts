/// Human-readable label for an encoded invocation `outcome` (plan H L3). The wire
/// value is `verdict:approve|revise|reject`, `error:<class>`, or `""` (in-flight).
/// Display-only: lowercase, plain, no exclamations or em-dashes (the impeccable
/// copy pass). Used by the history panel badges + the card's headline reason line.
export function outcomeLabel(outcome: string): string {
  if (outcome === "") return "in flight";
  if (outcome.startsWith("verdict:")) {
    const v = outcome.slice("verdict:".length);
    switch (v) {
      case "approve": return "approved";
      case "revise": return "revise requested";
      case "reject": return "rejected";
      default: return v;
    }
  }
  if (outcome.startsWith("error:")) {
    const c = outcome.slice("error:".length);
    switch (c) {
      case "rate_limited": return "rate limited";
      case "model_unavailable": return "model unavailable";
      case "spawn": return "could not start";
      case "no_result": return "no result";
      case "other": return "failed";
      default: return c.replace(/_/g, " ");
    }
  }
  return outcome;
}

/// Whether an encoded outcome is an operational error (vs a model verdict). Drives
/// the danger treatment on a history-panel badge.
export function isErrorOutcome(outcome: string): boolean {
  return outcome.startsWith("error:");
}
