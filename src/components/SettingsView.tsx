import { useState, type CSSProperties } from "react";
import type { UsageSnapshot } from "../ipc/usage";
import type { GitConfig, Project, WorktreeEntry } from "../ipc/workspace";
import { Button } from "./ui/Button";
import { FolderPickerField } from "./FolderPickerField";
import { formatTokens } from "../lib/cost";
import { useTheme } from "../hooks/useTheme";

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
  // Target repo (A5) — binds ${target_repo} for all teams' scope resolution.
  onSetTargetRepo: (projectId: string, targetRepo: string | null) => Promise<void>;
  // Skill sources (A4) — extra .claude roots scanned for prompt autocomplete.
  onSetSkillSources: (projectId: string, sources: string[]) => Promise<void>;
  // Worktree cleanup (S2)
  onListWorktrees: (projectId: string) => Promise<WorktreeEntry[]>;
  onRemoveWorktree: (projectId: string, path: string) => Promise<void>;
}

function basename(p: string): string {
  const parts = p.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

/** Per-project Target repo editor (A5). Seeds from the project's stored value;
 *  reuses FolderPickerField + a save action calling workspace_set_target_repo. */
function ProjectTargetRepo(props: {
  project: Project;
  onSave: (targetRepo: string | null) => Promise<void>;
}) {
  const { project, onSave } = props;
  const [val, setVal] = useState(project.target_repo ?? "");
  const [busy, setBusy] = useState(false);
  return (
    <div style={{ marginTop: 6, display: "flex", gap: 8, alignItems: "flex-end" }}>
      <div style={{ flex: 1 }}>
        <FolderPickerField label="Target repo" value={val} onChange={setVal} placeholder="~/projects/your-repo" disabled={busy} />
      </div>
      <Button
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          try {
            await onSave(val.trim() || null);
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy ? "saving…" : "save"}
      </Button>
    </div>
  );
}

/** Per-project Skill sources editor (A4). The global `~/.claude` row is shown
 *  locked/always-on; the operator may add one or more project `.claude` roots
 *  (via the native folder picker) or remove them. Each change persists the whole
 *  list through workspace_set_skill_sources. */
function ProjectSkillSources(props: {
  project: Project;
  onSave: (sources: string[]) => Promise<void>;
}) {
  const { project, onSave } = props;
  const [sources, setSources] = useState<string[]>(project.skill_sources ?? []);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);

  async function persist(next: string[]) {
    setSources(next);
    setBusy(true);
    try {
      await onSave(next);
    } finally {
      setBusy(false);
    }
  }

  async function add() {
    const path = draft.trim();
    if (!path || sources.includes(path)) {
      setDraft("");
      return;
    }
    setDraft("");
    await persist([...sources, path]);
  }

  return (
    <div style={{ marginTop: 6 }}>
      <span style={{ fontSize: 12, color: "var(--text-2)", display: "block", marginBottom: 4 }}>Skill sources</span>
      <ul aria-label={`skill sources for ${project.id}`} style={{ listStyle: "none", margin: 0, padding: 0, display: "grid", gap: 4 }}>
        <li style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 11, color: "var(--text-3)" }}>
          <span style={{ flex: 1, fontFamily: "var(--font-mono)" }}>~/.claude</span>
          <span aria-label="global source is always on" title="Always scanned" style={{ fontSize: 10, color: "var(--text-3)" }}>global · locked</span>
        </li>
        {sources.map((s) => (
          <li key={s} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 11, color: "var(--text-2)" }}>
            <span style={{ flex: 1, fontFamily: "var(--font-mono)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{s}</span>
            <Button size="sm" variant="ghost" disabled={busy} aria-label={`remove skill source ${s}`} onClick={() => persist(sources.filter((x) => x !== s))}>remove</Button>
          </li>
        ))}
      </ul>
      <div style={{ marginTop: 6, display: "flex", gap: 8, alignItems: "flex-end" }}>
        <div style={{ flex: 1 }}>
          <FolderPickerField label="Add a project .claude root" value={draft} onChange={setDraft} placeholder="~/project/.claude" disabled={busy} />
        </div>
        <Button disabled={busy || draft.trim() === ""} aria-label="add skill source" onClick={add}>add</Button>
      </div>
    </div>
  );
}

function ProjectWorktrees(props: {
  project: Project;
  onList: (projectId: string) => Promise<WorktreeEntry[]>;
  onRemove: (projectId: string, path: string) => Promise<void>;
}) {
  const { project, onList, onRemove } = props;
  const [open, setOpen] = useState(false);
  const [entries, setEntries] = useState<WorktreeEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmPath, setConfirmPath] = useState<string | null>(null);

  async function load() {
    setBusy(true);
    setError(null);
    try {
      setEntries(await onList(project.id));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function toggle() {
    const next = !open;
    setOpen(next);
    if (next && entries === null) await load();
  }

  async function remove(path: string) {
    setBusy(true);
    setError(null);
    try {
      await onRemove(project.id, path);
      setConfirmPath(null);
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ marginTop: 6 }}>
      <button
        onClick={toggle}
        aria-expanded={open}
        style={{ background: "none", border: "none", color: "var(--text-3)", fontSize: 11, cursor: "pointer", padding: 0 }}
      >
        {open ? "▾" : "▸"} worktrees
      </button>
      {open && (
        <div style={{ marginTop: 6, paddingLeft: 14 }}>
          {busy && entries === null && (
            <div style={{ fontSize: 11, color: "var(--text-3)" }}>loading…</div>
          )}
          {error && <div role="alert" style={{ fontSize: 11, color: "var(--danger)" }}>{error}</div>}
          {entries !== null && entries.length === 0 && (
            <div style={{ fontSize: 11, color: "var(--text-3)" }}>no worktrees to clean up.</div>
          )}
          {entries?.map((w) => (
            <div key={w.path} style={{ display: "flex", alignItems: "center", gap: 8, padding: "4px 0" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 12, color: "var(--text)" }}>{basename(w.path)}</div>
                <div style={{ fontSize: 11, color: "var(--text-3)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {w.branch || w.head || w.path} · not tied to an active task
                </div>
              </div>
              {confirmPath === w.path ? (
                <>
                  <Button variant="danger" disabled={busy} onClick={() => remove(w.path)}>confirm remove</Button>
                  <Button variant="ghost" disabled={busy} onClick={() => setConfirmPath(null)}>cancel</Button>
                </>
              ) : (
                <Button disabled={busy} onClick={() => setConfirmPath(w.path)}>remove</Button>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function SettingsView(props: SettingsViewProps) {
  const {
    usage, onSetBudget, onSetAutoMeter,
    apiKeyPresent, onSetApiKey, onClearApiKey,
    gitConfig, onSaveGitConfig,
    projects, activeProjectId, onRemoveProject, onSetTargetRepo, onSetSkillSources,
    onListWorktrees, onRemoveWorktree,
  } = props;

  const [theme, setTheme] = useTheme();
  const [budgetInput, setBudgetInput] = useState(String(usage?.window_budget ?? 190_000_000));
  const [saving, setSaving] = useState(false);
  const [autoMeter, setAutoMeter] = useState<boolean>(usage?.auto_meter_enabled ?? false);
  const [autoSaving, setAutoSaving] = useState(false);

  const [apiKeyInput, setApiKeyInput] = useState("");
  const [keyPresent, setKeyPresent] = useState(apiKeyPresent);
  const [keySaving, setKeySaving] = useState(false);

  const [gitName, setGitName] = useState(gitConfig.author_name);
  const [gitEmail, setGitEmail] = useState(gitConfig.author_email);
  const [gitSaving, setGitSaving] = useState(false);

  // Which project's delete is awaiting confirm (destructive: also wipes the
  // project's on-disk scaffolding, so it gets a danger + confirm step).
  const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);

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
          <Button variant={theme === "dark" ? "primary" : "default"} onClick={() => setTheme("dark")}>dark</Button>
          <Button variant={theme === "light" ? "primary" : "default"} onClick={() => setTheme("light")}>light</Button>
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
        <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-3)" }}>
          counts all tokens incl. cache. tune this so the % matches claude.ai for your plan — it's an estimate, not an exact mirror.
        </div>
        {usage && (
          <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-3)" }}>
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
          off by default. manual + reactive (rate-limit) braking stays on either way.
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
          {keyPresent ? "a key is stored in the keychain." : "no key stored. the anthropic-api runner falls back to the api_key_env var."}
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
          <div key={p.id} style={{ padding: "6px 0", borderBottom: "1px solid var(--border)" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 12, color: "var(--text)" }}>
                  {p.name}{p.id === activeProjectId ? " (active)" : ""}
                </div>
                <div style={{ fontSize: 11, color: "var(--text-3)" }}>{p.root_path}</div>
              </div>
              {confirmRemoveId === p.id ? (
                <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                  <Button
                    variant="danger"
                    aria-label={`confirm remove ${p.name}`}
                    onClick={async () => {
                      await onRemoveProject(p.id);
                      setConfirmRemoveId(null);
                    }}
                  >
                    confirm remove
                  </Button>
                  <Button variant="ghost" onClick={() => setConfirmRemoveId(null)}>cancel</Button>
                </div>
              ) : (
                <Button aria-label={`remove project ${p.name}`} onClick={() => setConfirmRemoveId(p.id)}>remove</Button>
              )}
            </div>
            {confirmRemoveId === p.id && (
              <div style={{ fontSize: 11, color: "var(--text-3)", marginTop: 4 }}>
                Also deletes this project's files (prompts, pipelines, artifacts,
                worktrees). The target repo is NOT touched.
              </div>
            )}
            <ProjectTargetRepo project={p} onSave={(path) => onSetTargetRepo(p.id, path)} />
            <ProjectSkillSources project={p} onSave={(sources) => onSetSkillSources(p.id, sources)} />
            <ProjectWorktrees project={p} onList={onListWorktrees} onRemove={onRemoveWorktree} />
          </div>
        ))}
      </div>
    </div>
  );
}
