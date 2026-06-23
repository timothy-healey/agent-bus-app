import { useState, type CSSProperties } from "react";
import type { UsageSnapshot } from "../ipc/usage";
import type { GitConfig, Project } from "../ipc/workspace";
import { Button } from "./ui/Button";
import { formatTokens } from "../lib/cost";

export interface SettingsViewProps {
  usage: UsageSnapshot | null;
  onSetBudget: (budget: number) => Promise<UsageSnapshot>;
  onSetAutoMeter: (enabled: boolean) => Promise<UsageSnapshot>;
  // Runners (R1 API key via keychain)
  apiKeyPresent: boolean;
  onSetApiKey: (key: string) => Promise<void>;
  onClearApiKey: () => Promise<void>;
  // Git author
  gitConfig: GitConfig;
  onSaveGitConfig: (name: string, email: string) => Promise<GitConfig>;
  // Projects
  projects: Project[];
  activeProjectId: string | null;
  onRemoveProject: (id: string) => Promise<void>;
}

type Theme = "dark" | "light";

function currentTheme(): Theme {
  return (document.documentElement.getAttribute("data-theme") as Theme) ?? "dark";
}

export function SettingsView(props: SettingsViewProps) {
  const {
    usage, onSetBudget, onSetAutoMeter,
    apiKeyPresent, onSetApiKey, onClearApiKey,
    gitConfig, onSaveGitConfig,
    projects, activeProjectId, onRemoveProject,
  } = props;

  const [theme, setTheme] = useState<Theme>(currentTheme());
  const [budgetInput, setBudgetInput] = useState(String(usage?.window_budget ?? 2_600_000));
  const [saving, setSaving] = useState(false);
  const [autoMeter, setAutoMeter] = useState<boolean>(usage?.auto_meter_enabled ?? false);
  const [autoSaving, setAutoSaving] = useState(false);

  const [apiKeyInput, setApiKeyInput] = useState("");
  const [keyPresent, setKeyPresent] = useState(apiKeyPresent);
  const [keySaving, setKeySaving] = useState(false);

  const [gitName, setGitName] = useState(gitConfig.author_name);
  const [gitEmail, setGitEmail] = useState(gitConfig.author_email);
  const [gitSaving, setGitSaving] = useState(false);

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

  async function saveApiKey() {
    if (apiKeyInput.trim() === "") return;
    setKeySaving(true);
    try {
      await onSetApiKey(apiKeyInput);
      setApiKeyInput("");
      setKeyPresent(true);
    } finally {
      setKeySaving(false);
    }
  }

  async function clearApiKey() {
    setKeySaving(true);
    try {
      await onClearApiKey();
      setKeyPresent(false);
    } finally {
      setKeySaving(false);
    }
  }

  async function saveGit() {
    setGitSaving(true);
    try {
      const c = await onSaveGitConfig(gitName, gitEmail);
      setGitName(c.author_name);
      setGitEmail(c.author_email);
    } finally {
      setGitSaving(false);
    }
  }

  const section: CSSProperties = { maxWidth: 560, marginBottom: "var(--sp-10)" };
  const h: CSSProperties = { fontSize: 11, color: "var(--text-3)", textTransform: "lowercase", letterSpacing: "0.04em", marginBottom: "var(--sp-3)", borderBottom: "1px solid var(--border)", paddingBottom: 4 };
  const label: CSSProperties = { fontSize: 12, color: "var(--text-2)", display: "block", marginBottom: 6 };
  const input: CSSProperties = { background: "var(--surface)", border: "1px solid var(--border)", color: "var(--text)", borderRadius: "var(--r-sm)", padding: "5px 10px", fontSize: 12, fontFamily: "inherit", width: 260 };

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
            style={{ ...input, width: 160, fontVariantNumeric: "tabular-nums" }} />
          <Button variant="primary" disabled={saving} onClick={saveBudget}>save budget</Button>
        </div>
        {usage && (
          <div style={{ marginTop: 8, fontSize: 11, color: "var(--text-3)" }}>
            currently {formatTokens(usage.window_total)} of {formatTokens(usage.window_budget)} used this window.
          </div>
        )}
        <label style={{ ...label, display: "flex", alignItems: "center", gap: 8, marginTop: 16, marginBottom: 0, cursor: "pointer" }}>
          <input type="checkbox" checked={autoMeter} disabled={autoSaving}
            onChange={(e) => toggleAutoMeter(e.target.checked)} aria-label="auto-brake"
            style={{ accentColor: "var(--accent)", cursor: "pointer" }} />
          auto-brake when the window crosses the threshold
        </label>
        <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-3)" }}>
          off by default — manual + reactive (rate-limit) braking stays on either way.
        </div>
      </div>

      <div style={section}>
        <div style={h}>runners</div>
        <label style={label} htmlFor="api-key">anthropic api key (stored in the OS keychain)</label>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <input id="api-key" type="password" value={apiKeyInput} aria-label="api key"
            placeholder={keyPresent ? "•••••••• (stored)" : "sk-ant-..."}
            onChange={(e) => setApiKeyInput(e.target.value)} style={input} />
          <Button variant="primary" disabled={keySaving} onClick={saveApiKey}>save key</Button>
          {keyPresent && <Button disabled={keySaving} onClick={clearApiKey}>clear</Button>}
        </div>
        <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-3)" }}>
          {keyPresent ? "a key is stored in the keychain." : "no key stored — the anthropic-api runner falls back to the api_key_env var."}
        </div>
      </div>

      <div style={section}>
        <div style={h}>git</div>
        <label style={label} htmlFor="git-name">author name</label>
        <input id="git-name" aria-label="author name" value={gitName}
          onChange={(e) => setGitName(e.target.value)} style={input} />
        <label style={{ ...label, marginTop: 10 }} htmlFor="git-email">author email</label>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <input id="git-email" aria-label="author email" value={gitEmail}
            onChange={(e) => setGitEmail(e.target.value)} style={input} />
          <Button variant="primary" disabled={gitSaving} onClick={saveGit}>save</Button>
        </div>
        <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-3)" }}>
          used for commits workers make in worktrees (applied when worker-commits land).
        </div>
      </div>

      <div style={section}>
        <div style={h}>projects</div>
        {projects.length === 0 && (
          <div style={{ fontSize: 11, color: "var(--text-3)" }}>no projects yet.</div>
        )}
        {projects.map((p) => (
          <div key={p.id} style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 0", borderBottom: "1px solid var(--border)" }}>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 12, color: "var(--text)" }}>
                {p.name}{p.id === activeProjectId ? " (active)" : ""}
              </div>
              <div style={{ fontSize: 11, color: "var(--text-3)" }}>{p.root_path}</div>
            </div>
            <Button onClick={() => onRemoveProject(p.id)}>remove</Button>
          </div>
        ))}
      </div>
    </div>
  );
}
