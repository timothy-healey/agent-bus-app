import type { UsageSnapshot } from "../ipc/usage";
import { formatTokens } from "../lib/cost";
import { bandColorVar, brakeEta, formatBurn, resetCountdown, windowLine } from "../lib/usageMeter";

export interface UsageMeterProps {
  snapshot: UsageSnapshot | null;
  /// Epoch seconds reference for the brake-ETA projection (est_brake_at is an
  /// absolute timestamp). Defaults to the wall clock; tests pass a fixed value.
  now?: number;
}

// The one sanctioned gradient (DESIGN.md §Usage meter): --running -> --warn ->
// --danger at 0% / 60% / 90% of the 100px track. The fill reveals it left-to-
// right by width; the gradient itself is painted across the full track so the
// colour at any fill width is the true band colour for that position.
const GRADIENT =
  "linear-gradient(90deg, var(--running) 0%, var(--warn) 60%, var(--danger) 90%)";

export function UsageMeter({ snapshot, now = Math.floor(Date.now() / 1000) }: UsageMeterProps) {
  const pct = snapshot ? Math.round(snapshot.window_pct * 100) : 0;
  const band = snapshot?.band ?? "safe";
  const color = bandColorVar(band);
  const braked = snapshot?.braked ?? false;

  return (
    <div
      data-testid="usage-meter"
      tabIndex={0}
      role="group"
      aria-label={`usage ${pct}% of window`}
      style={{
        position: "relative",
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-3)",
        padding: "4px 12px",
        background: "var(--bg-2)",
        border: `1px solid ${band === "hot" || braked ? "var(--danger)" : "var(--border)"}`,
        borderRadius: "var(--r-md)",
        opacity: braked ? 0.6 : 1,
        cursor: "default",
      }}
      className="usage-meter-hoverable"
    >
      <div
        data-testid="usage-bar"
        role="progressbar"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
        style={{ width: 100, height: 6, background: "var(--surface-3)", borderRadius: 3, overflow: "hidden" }}
      >
        {/* Clip the full-track gradient to the current fill width. */}
        <div style={{ height: "100%", width: `${Math.min(pct, 100)}%`, overflow: "hidden" }}>
          <div data-testid="usage-bar-fill" style={{ height: "100%", width: 100, background: GRADIENT }} />
        </div>
      </div>

      <div style={{ fontSize: 11, color: "var(--text-2)", display: "flex", alignItems: "center", gap: 6, fontVariantNumeric: "tabular-nums" }}>
        <span style={{ color, fontWeight: 500 }}>{pct}%</span>
        {braked ? (
          <span style={{ color: "var(--text-3)", fontSize: 10.5 }}>
            · {snapshot?.reset_in_secs != null ? resetCountdown(snapshot.reset_in_secs) : "braked"}
          </span>
        ) : (
          <span style={{ color: "var(--text-3)", fontSize: 10.5 }}>
            · {snapshot ? windowLine(snapshot.window_secs, snapshot.window_total) : "—"}
          </span>
        )}
      </div>

      {!braked && snapshot && (
        <>
          <span style={{ color: "var(--text-4)", fontSize: 11 }}>·</span>
          <div style={{ fontSize: 11, color: "var(--text-3)", fontVariantNumeric: "tabular-nums" }}>
            burn <span style={{ color: band === "hot" ? "var(--danger)" : "var(--text-2)" }}>{formatBurn(snapshot.burn_per_min)}</span>
          </div>
        </>
      )}

      {snapshot && (
        <div
          className="usage-tooltip"
          style={{
            position: "absolute",
            top: "calc(100% + 8px)",
            right: 0,
            minWidth: 240,
            background: "var(--surface-3)",
            border: "1px solid var(--border-2)",
            borderRadius: "var(--r-md)",
            padding: "12px 14px",
            boxShadow: "var(--shadow-card)",
            zIndex: 20,
            textAlign: "left",
          }}
        >
          <div style={{ fontSize: 11.5, color: "var(--text)", fontWeight: 500, marginBottom: 10 }}>
            claude usage · rolling {Math.round(snapshot.window_secs / 3600)}h window
          </div>
          <Row label="tokens this window" value={`${formatTokens(snapshot.window_total)} / ${formatTokens(snapshot.window_budget)}`} />
          {!braked && (
            <Row label="burn rate (1-min avg)" value={`${formatTokens(Math.round(snapshot.burn_per_min))} tok/min`} />
          )}
          {!braked && brakeEta(snapshot.est_brake_at, now) && (
            <Row label="brake at budget" value={`in ${brakeEta(snapshot.est_brake_at, now)}`} />
          )}
          {!braked && snapshot.reset_in_secs != null && (
            <Row label="window resets in" value={resetCountdown(snapshot.reset_in_secs)} />
          )}
          {/* Deferred (needs backend plumbing, see plan FE3): 10-min burn avg and
              per-team burn/effort are NOT on UsageSnapshot — do not fake them. */}
          <div style={{ marginTop: 10, paddingTop: 10, borderTop: "1px solid var(--border)" }}>
            <div style={{ color: "var(--text-3)", fontSize: 10.5, marginBottom: 6 }}>by team</div>
            {snapshot.by_team.length === 0 ? (
              <div style={{ color: "var(--text-4)", fontSize: 11 }}>no usage yet</div>
            ) : (
              snapshot.by_team.map((t) => (
                <div key={t.team_id} style={{ display: "flex", justifyContent: "space-between", padding: "2px 0", fontSize: 11, color: "var(--text-3)" }}>
                  <span>{t.team_id}</span>
                  <span style={{ color: "var(--text-2)" }}>{formatTokens(t.tokens)}</span>
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
