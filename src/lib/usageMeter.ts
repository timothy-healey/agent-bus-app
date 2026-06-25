import { formatTokens } from "./cost";
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

export function windowLine(windowSecs: number, total: number): string {
  const hours = Math.round(windowSecs / 3600);
  return `${hours}h window · ${formatTokens(total)} tok`;
}

export function formatBurn(burnPerMin: number): string {
  return `${formatTokens(Math.round(burnPerMin))}/min`;
}

export function resetCountdown(secs: number): string {
  const totalMin = Math.ceil(secs / 60);
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  return h > 0 ? `↻ ${h}h ${m}m` : `↻ ${m}m`;
}

/// Human "time until the window crosses the brake threshold", from the snapshot's
/// est_brake_at (an ABSOLUTE unix-seconds timestamp, window.rs::est_brake_at) and
/// a `now` reference (unix secs). Returns null when there is no projection
/// (est_brake_at == null, e.g. zero burn or already braked) or it is already past.
export function brakeEta(estBrakeAt: number | null, now: number): string | null {
  if (estBrakeAt == null) return null;
  const secs = estBrakeAt - now;
  if (secs <= 0) return null;
  const totalMin = Math.round(secs / 60);
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  return h > 0 ? `~${h}h ${m}min` : `~${m}min`;
}
