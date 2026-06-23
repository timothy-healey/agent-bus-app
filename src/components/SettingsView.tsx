import { useState, type CSSProperties } from "react";
import type { UsageSnapshot } from "../ipc/usage";
import { Button } from "./ui/Button";
import { formatTokens } from "../lib/cost";

export interface SettingsViewProps {
  usage: UsageSnapshot | null;
  onSetBudget: (budget: number) => Promise<UsageSnapshot>;
  onSetAutoMeter: (enabled: boolean) => Promise<UsageSnapshot>;
}

type Theme = "dark" | "light";

function currentTheme(): Theme {
  return (document.documentElement.getAttribute("data-theme") as Theme) ?? "dark";
}

export function SettingsView({ usage, onSetBudget, onSetAutoMeter }: SettingsViewProps) {
  const [theme, setTheme] = useState<Theme>(currentTheme());
  const [budgetInput, setBudgetInput] = useState(String(usage?.window_budget ?? 2_600_000));
  const [saving, setSaving] = useState(false);
  const [autoMeter, setAutoMeter] = useState<boolean>(usage?.auto_meter_enabled ?? false);
  const [autoSaving, setAutoSaving] = useState(false);

  async function toggleAutoMeter(next: boolean) {
    setAutoMeter(next);
    setAutoSaving(true);
    try {
      const s = await onSetAutoMeter(next);
      setAutoMeter(s.auto_meter_enabled);
    } finally {
      setAutoSaving(false);
    }
  }

  function applyTheme(next: Theme) {
    document.documentElement.setAttribute("data-theme", next);
    setTheme(next);
  }

  async function saveBudget() {
    const n = Number(budgetInput);
    if (!Number.isFinite(n) || n <= 0) return;
    setSaving(true);
    try {
      await onSetBudget(n);
    } finally {
      setSaving(false);
    }
  }

  const section: CSSProperties = { maxWidth: 560, marginBottom: "var(--sp-10)" };
  const h: CSSProperties = { fontSize: 11, color: "var(--text-3)", textTransform: "lowercase", letterSpacing: "0.04em", marginBottom: "var(--sp-3)", borderBottom: "1px solid var(--border)", paddingBottom: 4 };
  const label: CSSProperties = { fontSize: 12, color: "var(--text-2)", display: "block", marginBottom: 6 };

  return (
    <div style={{ padding: "var(--sp-8)" }}>
      <div style={section}>
        <div style={h}>general</div>
        <span style={label}>theme</span>
        <div style={{ display: "flex", gap: 8 }}>
          <Button variant={theme === "dark" ? "primary" : "default"} onClick={() => applyTheme("dark")}>dark</Button>
          <Button variant={theme === "light" ? "primary" : "default"} onClick={() => applyTheme("light")}>light</Button>
        </div>
      </div>

      <div style={section}>
        <div style={h}>usage</div>
        <label style={label} htmlFor="budget">window budget (tokens / 5h)</label>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <input id="budget" value={budgetInput} onChange={(e) => setBudgetInput(e.target.value)}
            style={{ background: "var(--surface)", border: "1px solid var(--border)", color: "var(--text)", borderRadius: "var(--r-sm)", padding: "5px 10px", fontSize: 12, fontFamily: "inherit", width: 160, fontVariantNumeric: "tabular-nums" }} />
          <Button variant="primary" disabled={saving} onClick={saveBudget}>save budget</Button>
        </div>
        {usage && (
          <div style={{ marginTop: 8, fontSize: 11, color: "var(--text-3)" }}>
            currently {formatTokens(usage.window_total)} of {formatTokens(usage.window_budget)} used this window.
          </div>
        )}

        <label style={{ ...label, display: "flex", alignItems: "center", gap: 8, marginTop: 16, marginBottom: 0, cursor: "pointer" }}>
          <input
            type="checkbox"
            checked={autoMeter}
            disabled={autoSaving}
            onChange={(e) => toggleAutoMeter(e.target.checked)}
            aria-label="auto-brake"
            style={{ accentColor: "var(--accent)", cursor: "pointer" }}
          />
          auto-brake when the window crosses the threshold
        </label>
        <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-3)" }}>
          off by default — manual + reactive (rate-limit) braking stays on either way.
        </div>
      </div>

      <div style={{ ...section, color: "var(--text-3)", fontSize: 11 }}>
        runners (api key), git worktree base, and project management arrive in v1.1.
      </div>
    </div>
  );
}
