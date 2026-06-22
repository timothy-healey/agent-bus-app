export type CostBand = "green" | "amber" | "red";

/// Colour band for a token count, per the card-glance-v2 mockup:
/// green < 20k, amber 20k..100k, red > 100k.
export function costBand(tokens: number): CostBand {
  if (tokens < 20_000) return "green";
  if (tokens <= 100_000) return "amber";
  return "red";
}

/// CSS var the band maps to (state dots / cost label colour).
export const costColorVar: Record<CostBand, string> = {
  green: "var(--running)",
  amber: "var(--accent)",
  red: "var(--danger)",
};

/// Compact token formatting: 999 -> "999", 47000 -> "47k", 1.2M -> "1.2M".
export function formatTokens(tokens: number): string {
  if (tokens < 1_000) return String(tokens);
  if (tokens < 1_000_000) return `${Math.round(tokens / 1_000)}k`;
  return `${(tokens / 1_000_000).toFixed(1)}M`;
}
