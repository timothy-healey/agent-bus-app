/// Curated list of known current Claude model IDs (G6). There is no
/// subscription model-list API, so the node-drawer's model SELECTOR is driven
/// by this constant (opus/sonnet/haiku families) rather than false
/// pre-validation. A free-text override is still allowed and flagged
/// "unverified"; availability is confirmed where it fails (the runner's
/// model-unavailable class) or via the authoring-time `test_model` probe.

export interface ClaudeModel {
  id: string;
  label: string;
  family: "opus" | "sonnet" | "haiku";
}

/// Known current model IDs, newest-first within each family. Use the exact ID
/// strings as-is (no date suffixes).
export const CLAUDE_MODELS: ClaudeModel[] = [
  { id: "claude-opus-4-8", label: "Claude Opus 4.8", family: "opus" },
  { id: "claude-opus-4-7", label: "Claude Opus 4.7", family: "opus" },
  { id: "claude-opus-4-6", label: "Claude Opus 4.6", family: "opus" },
  { id: "claude-sonnet-4-6", label: "Claude Sonnet 4.6", family: "sonnet" },
  { id: "claude-haiku-4-5", label: "Claude Haiku 4.5", family: "haiku" },
];

/// True when `model` is one of the curated known IDs (selector entry). A model
/// outside this set is a free-text override and should be flagged "unverified".
export function isKnownModel(model: string): boolean {
  return CLAUDE_MODELS.some((m) => m.id === model);
}
