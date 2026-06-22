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
