import type { LimitView, UsageSnapshot } from "../ipc/usage";
import { formatTokens } from "../lib/cost";
import { asOf, bandColorVar, formatUsd, isStale, resetCountdown } from "../lib/usageMeter";

export interface UsageMeterProps {
  snapshot: UsageSnapshot | null;
  /// Epoch seconds reference for staleness. Defaults to the wall clock; tests pass a fixed value.
  now?: number;
}

// The one sanctioned gradient (DESIGN.md §Usage meter): --running -> --warn ->
// --danger at 0% / 60% / 90% of the 100px track.
const GRADIENT =
  "linear-gradient(90deg, var(--running) 0%, var(--warn) 60%, var(--danger) 90%)";

export function UsageMeter({ snapshot, now = Math.floor(Date.now() / 1000) }: UsageMeterProps) {
  const session = snapshot?.session ?? null;
  const pct = session ? Math.round(session.utilization_pct) : 0;
  const band = snapshot?.band ?? "safe";
  const color = bandColorVar(band);
  const braked = snapshot?.braked ?? false;
  const observedAt = snapshot?.observed_at ?? null;
  const unavailable = snapshot != null && !snapshot.available;
  const showAge = observedAt != null && (unavailable || isStale(observedAt, now));

  return (
    <div
      data-testid="usage-meter"
      tabIndex={0}
      role="group"
      aria-label={session ? `usage ${pct}% of session limit` : "usage unavailable"}
      style={{
        position: "relative", display: "flex", alignItems: "center", gap: "var(--sp-3)",
        padding: "4px 12px", background: "var(--bg-2)",
        border: `1px solid ${band === "hot" || braked ? "var(--danger)" : "var(--border)"}`,
        borderRadius: "var(--r-md)", opacity: braked ? 0.6 : 1, cursor: "default",
      }}
      className="usage-meter-hoverable"
    >
      <div data-testid="usage-bar" role="progressbar" {...(session ? { "aria-valuenow": pct } : { "aria-valuetext": "unknown" })} aria-valuemin={0} aria-valuemax={100}
        style={{ width: 100, height: 6, background: "var(--surface-3)", borderRadius: 3, overflow: "hidden" }}>
        <div style={{ height: "100%", width: `${Math.min(pct, 100)}%`, overflow: "hidden" }}>
          <div data-testid="usage-bar-fill" style={{ height: "100%", width: 100, background: GRADIENT }} />
        </div>
      </div>

      <div data-testid="usage-headline" style={{ fontSize: 11, color: "var(--text-2)", display: "flex", alignItems: "center", gap: 6, fontVariantNumeric: "tabular-nums" }}>
        {session ? <span style={{ color, fontWeight: 500 }}>{pct}%</span> : <span style={{ color: "var(--text-3)" }}>—</span>}
        {unavailable && observedAt != null && <span style={{ color: "var(--text-3)", fontSize: 10.5 }}>— usage unavailable</span>}
        {showAge && observedAt != null && <span style={{ color: "var(--text-3)", fontSize: 10.5 }}>· {asOf(observedAt)}</span>}
        {!showAge && session && (
          <span style={{ color: "var(--text-3)", fontSize: 10.5 }}>· {resetCountdown(session.resets_in_secs)}</span>
        )}
      </div>

      {snapshot && (
        <div className="usage-tooltip" style={{
          position: "absolute", top: "calc(100% + 8px)", right: 0, minWidth: 260,
          background: "var(--surface-3)", border: "1px solid var(--border-2)", borderRadius: "var(--r-md)",
          padding: "12px 14px", boxShadow: "var(--shadow-card)", zIndex: 20, textAlign: "left",
        }}>
          <div style={{ fontSize: 11.5, color: "var(--text)", fontWeight: 500, marginBottom: 10 }}>claude plan usage</div>
          {[snapshot.session, snapshot.weekly, ...snapshot.model_scoped]
            .filter((l): l is LimitView => l != null)
            .map((l) => (
              <Row key={l.label} label={l.label} value={`${Math.round(l.utilization_pct)}% · ${resetCountdown(l.resets_in_secs)}`} />
            ))}
          {observedAt != null && showAge && <Row label="reading" value={unavailable ? `${asOf(observedAt)} · unavailable` : asOf(observedAt)} />}
          <Row label="auto-brake" value={snapshot.auto_meter_enabled ? "on" : "off"} />
          <div style={{ marginTop: 10, paddingTop: 10, borderTop: "1px solid var(--border)" }}>
            <div style={{ color: "var(--text-3)", fontSize: 10.5, marginBottom: 6 }}>cost this 5h (list price)</div>
            {snapshot.by_team.length === 0 ? (
              <div style={{ color: "var(--text-4)", fontSize: 11 }}>no usage yet</div>
            ) : (
              snapshot.by_team.map((t) => (
                <div key={t.team_id} style={{ display: "flex", justifyContent: "space-between", padding: "2px 0", fontSize: 11, color: "var(--text-3)" }}>
                  <span>{t.team_id}</span>
                  <span style={{ color: "var(--text-2)" }}>{formatUsd(t.cost_usd)} · {formatTokens(t.tokens)} tok</span>
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, color: "var(--text-3)", padding: "3px 0" }}>
      <span>{label}</span>
      <span style={{ color: "var(--text-2)", fontVariantNumeric: "tabular-nums" }}>{value}</span>
    </div>
  );
}
