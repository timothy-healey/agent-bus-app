/// Compact age between an epoch-seconds timestamp and `now` (epoch seconds).
/// Deterministic: `now` is passed in, never read from the clock.
export function formatAge(createdAtSecs: number, nowSecs: number): string {
  let d = Math.max(0, Math.floor(nowSecs - createdAtSecs));
  if (d < 60) return `${d}s`;
  if (d < 3600) return `${Math.floor(d / 60)}m ${d % 60}s`;
  if (d < 86400) return `${Math.floor(d / 3600)}h ${Math.floor((d % 3600) / 60)}m`;
  return `${Math.floor(d / 86400)}d ${Math.floor((d % 86400) / 3600)}h`;
}
