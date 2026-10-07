import type { ThresholdBand } from "../ipc/usage";

export function bandColorVar(band: ThresholdBand): string {
  switch (band) {
    case "safe":
      return "var(--running)";
    case "warn":
      return "var(--warn)";
    case "hot":
    case "braked":
      return "var(--danger)";
  }
}

export function resetCountdown(secs: number): string {
  const totalMin = Math.ceil(secs / 60);
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  return h > 0 ? `↻ ${h}h ${m}m` : `↻ ${m}m`;
}

/// A reading older than this shows its age.
const STALE_SECS = 180;

export function isStale(observedAt: number | null, now: number): boolean {
  return observedAt == null || now - observedAt > STALE_SECS;
}

export function asOf(observedAt: number): string {
  const d = new Date(observedAt * 1000);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `as of ${hh}:${mm}`;
}

export function formatUsd(n: number): string {
  if (n > 0 && n < 0.01) return "<$0.01";
  return `$${n.toFixed(2)}`;
}
