import type React from "react";
import type { CSSProperties } from "react";
import type { DraftPipeline, DraftTeam, SimpleGrant } from "../../ipc/pipeline";
import { regenerateTeamPrompt } from "../../ipc/pipeline";
import type { SkillEntry } from "../../ipc/skills";
import { useEffect, useId, useState } from "react";
import { listPlugins, type PluginInfo } from "../../ipc/plugins";
import { Drawer } from "../../components/ui/Drawer";
import { Button } from "../../components/ui/Button";
import { SkillAutocomplete } from "./SkillAutocomplete";
import { FileTreePicker } from "../../components/FileTreePicker";
import { InfoTip } from "../../components/InfoTip";
import { toRepoRelative } from "../../components/fileTree";
import { findModel, modelLabel, sourceLabel } from "../../ipc/models";
import { useModelList } from "../../hooks/useModelList";
import {
  renameTeam,
  setPromptBody,
  setTeamModel,
  setTeamEffort,
  setTeamGrant,
  setTeamBashPatterns,
  bashPatterns,
  setTeamPlugin,
  addTeamPlugin,
  setTeamReads,
  setTeamWrites,
  setTeamRole,
  setTeamStoreCapacity,
  setTeamWorkers,
  setJoinQuorum,
  setJoinCancelOnReject,
} from "../draft";

/// The node drawer (vet F1): a generic right-side container whose CONTENTS are
/// chosen by the selected node's kind. Header = "kind · id". Team → full config
/// (name, prompt, Runner, Scope, Role, Scale, Store). Gate → label / downstream.
/// Join → Lanes / Quorum / Early-cancel. Fork → lanes. All edits go through the
/// pure draft mutators so DraftPipeline stays the single source of truth.

interface NodeDrawerProps {
  draft: DraftPipeline;
  /// id of the selected node, or null when nothing is selected (drawer closed).
  selectedId: string | null;
  onChange: (d: DraftPipeline) => void;
  onClose: () => void;
  /// A4 — discovered skills for the Team prompt's `/`-autocomplete. Default empty.
  skills?: SkillEntry[];
  /// G9 — delete the selected node (routes through `removeNode` at the host). When
  /// absent the header delete button is hidden.
  onDelete?: (id: string) => void;
  /// G7 — absolute path of the project's target repo. When present, the Scope
  /// reads/writes fields offer the in-app FileTreePicker rooted here (multi-
  /// select, stored repo-relative); the comma-separated text stays as the
  /// fallback. Absent (e.g. mid-create before a repo is bound) = text only.
  targetRepo?: string | null;
  /// G5 — the Design Session dialogue session id. When present, a Team's drawer
  /// offers a "regenerate prompt" action that runs the Step::Prompts design-session
  /// logic for that one team over this session. Absent → the action is hidden.
  sessionId?: string;
}

type Kind = "team" | "gate" | "fork" | "join" | "escalation" | "store" | "unknown";

/// The G1 store node id is synthetic: `store:<team-id>`.
const STORE_PREFIX = "store:";
function storeTeamId(id: string): string | null {
  return id.startsWith(STORE_PREFIX) ? id.slice(STORE_PREFIX.length) : null;
}

function kindOf(draft: DraftPipeline, id: string): Kind {
  if (storeTeamId(id)) return "store";
  if (draft.teams.some((t) => t.id === id)) return "team";
  if (draft.gates.some((g) => g.id === id)) return "gate";
  if (draft.forks.some((f) => f.id === id)) return "fork";
  if (draft.joins.some((j) => j.id === id)) return "join";
  if (draft.escalations.some((e) => e.id === id)) return "escalation";
  return "unknown";
}

export function NodeDrawer({ draft, selectedId, onChange, onClose, skills = [], onDelete, targetRepo, sessionId }: NodeDrawerProps) {
  const open = selectedId != null;
  const kind = selectedId ? kindOf(draft, selectedId) : "unknown";
  // A store node shows its OWNING team id (not the synthetic `store:<id>`).
  const displayId = selectedId ? (storeTeamId(selectedId) ?? selectedId) : "";
  // Store nodes are derived (projected) — they are never deletable.
  const deletable = kind !== "store";

  return (
    <Drawer open={open} onClose={onClose} label={selectedId ? `${kind} · ${displayId}` : "node"}>
      {selectedId && (
        <div style={body}>
          <header style={{ marginBottom: "var(--sp-5)", display: "flex", alignItems: "center", justifyContent: "space-between", gap: "var(--sp-3)", paddingRight: "var(--sp-7)" }}>
            <div style={{ fontSize: "var(--ts-sm)", color: "var(--text-3)", textTransform: "capitalize", letterSpacing: "0.04em" }}>
              {kind} · <span style={{ color: "var(--text-2)" }}>{displayId}</span>
            </div>
            {onDelete && deletable && (
              <Button variant="danger" size="sm" onClick={() => onDelete(selectedId)} aria-label={`delete ${selectedId}`}>
                Delete
              </Button>
            )}
          </header>
          {kind === "team" && <TeamEditor draft={draft} id={selectedId} onChange={onChange} skills={skills} targetRepo={targetRepo} sessionId={sessionId} />}
          {kind === "store" && <StoreEditor draft={draft} teamId={displayId} onChange={onChange} />}
          {kind === "gate" && <GateEditor draft={draft} id={selectedId} onChange={onChange} />}
          {kind === "join" && <JoinEditor draft={draft} id={selectedId} onChange={onChange} />}
          {kind === "fork" && <ForkEditor draft={draft} id={selectedId} />}
          {kind === "escalation" && (
            <p style={{ color: "var(--text-3)", fontSize: "var(--ts-base)" }}>
              An escalation is a terminal node. Route a reviewer's reject edge here.
            </p>
          )}
        </div>
      )}
    </Drawer>
  );
}

/// G1 — the store node drawer. A store is a derived projection of its team's
/// bounded input buffer, so its only authoring control edits the owning team's
/// `store.capacity` (via setTeamStoreCapacity). Live occupancy is a runtime/board
/// concern (④e), not authored here.
function StoreEditor({ draft, teamId, onChange }: { draft: DraftPipeline; teamId: string; onChange: (d: DraftPipeline) => void }) {
  const t = draft.teams.find((x) => x.id === teamId);
  if (!t) return null;
  return (
    <div style={{ display: "grid", gap: "var(--sp-4)" }}>
      <p style={{ color: "var(--text-3)", fontSize: "var(--ts-base)", margin: 0 }}>
        The bounded input buffer for <span style={{ color: "var(--text-2)" }}>{t.name || teamId}</span>. The WIP limit
        back-pressures upstream when this many tasks are queued.
      </p>
      <fieldset style={group}>
        <TipLegend text={HELP.store}>Store capacity</TipLegend>
        <Field label="WIP limit">
          <input
            type="number"
            min={1}
            aria-label={`store capacity for ${teamId}`}
            value={t.store?.capacity ?? 8}
            onChange={(e) => onChange(setTeamStoreCapacity(draft, teamId, Number(e.target.value) || 1))}
            style={inp}
          />
        </Field>
      </fieldset>
    </div>
  );
}

function Field({ label, children }: { label: React.ReactNode; children: React.ReactNode }) {
  return (
    <label style={lbl}>
      <span style={{ display: "block", marginBottom: "var(--sp-1)" }}>{label}</span>
      {children}
    </label>
  );
}

/// G8 help copy for the unclear authoring fields. Concise, action-oriented.
const HELP = {
  reads: "Paths the agent may READ (relative to the project's target repo). Add an artifacts path so it can see upstream work. Leave empty to read nothing extra.",
  writes: "Paths the agent may WRITE. A producer needs its artifacts dir; a reviewer that only judges can have none.",
  grants: "Every worker can read, search, edit and use skills. A tool you don't grant here is removed, so the worker never sees it. Limits inside a granted tool are denied by rule, and each denial shows on the task. Remote git lets the worker push, fetch and use gh when the task asks for it; it needs a model with auto mode. Force pushes, pushes to main or master, merging PRs and creating or deleting repos stay forbidden.",
  bashPatterns: "Shell commands this team may run without the full Bash grant, as CLI patterns, e.g. git diff:*",
  plugins: "Claude Code plugins loaded for this team's workers. Workers never load your own plugins, so a skill works only if its plugin is listed here. Picking a skill in the prompt adds its plugin.",
  role: "producer = does work and hands off on approve. reviewer = judges upstream work and emits approve / revise / reject.",
  runner: "Which Claude runs this team and how hard it reasons. Models and effort levels come from your installed Claude CLI. An API-key env var name is only used by the anthropic-api runner.",
  scale: "Worker concurrency for this team: minimum kept warm and maximum it can burst to.",
  store: "Bounded input buffer (WIP limit) — how many tasks can queue for this team before upstream back-pressures.",
  quorum: "Proceed once N of the M lanes approve (N-of-M). Blank means all must approve. A set quorum governs success and overrides early-cancel.",
  cancel: "Resolve the join to needs-human the instant one lane fails, cancelling the rest. Ignored while a quorum is set (the quorum decides success).",
} as const;

/// A fieldset legend with an inline G8 InfoTip.
function TipLegend({ text, children }: { text: string; children: React.ReactNode }) {
  return (
    <legend style={{ ...legend, display: "inline-flex", alignItems: "center", gap: "var(--sp-2)" }}>
      {children}
      <InfoTip text={text} label={`help for ${String(children)}`} />
    </legend>
  );
}

/// An inline label + G8 InfoTip (for individual Field labels).
function LabelTip({ text, children }: { text: string; children: React.ReactNode }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "var(--sp-2)" }}>
      {children}
      <InfoTip text={text} label={`help for ${String(children)}`} />
    </span>
  );
}

/// Model and Effort pickers over the CLI's model list. Model offers the list's
/// entries (aliases show what they resolve to); a saved model the list lacks
/// shows as "not available". Effort offers Default plus the selected model's
/// levels. Switching to a model that lacks the saved level snaps it to Default
/// and says so.
function RunnerPickers({ draft, team, onChange }: { draft: DraftPipeline; team: DraftTeam; onChange: (d: DraftPipeline) => void }) {
  const { list, refreshing, refresh } = useModelList();
  const [note, setNote] = useState<string | null>(null);
  const id = team.id;
  const model = team.runner.model;
  const effort = team.runner.effort;
  const option = findModel(list, model);
  const levels = option?.effort_levels ?? [];
  const modelMissing = !option;
  const effortMissing = effort !== undefined && !levels.includes(effort);

  function pickModel(value: string) {
    const { draft: next, snapped } = setTeamModel(draft, id, value, list);
    onChange(next);
    setNote(snapped ? `${snapped} isn't supported by ${value}; effort reset to Default.` : null);
  }

  return (
    <>
      <Field label="Model">
        <select aria-label={`model for ${id}`} value={model} onChange={(e) => pickModel(e.target.value)} style={inp}>
          {modelMissing && <option value={model}>{list ? `${model} — not available` : model}</option>}
          {list?.models.map((m) => (
            <option key={m.value} value={m.value}>{modelLabel(m)}</option>
          ))}
        </select>
      </Field>
      <Field label="Effort">
        <select
          aria-label={`effort for ${id}`}
          value={effort ?? ""}
          onChange={(e) => {
            onChange(setTeamEffort(draft, id, e.target.value || undefined));
            setNote(null);
          }}
          style={inp}
        >
          <option value="">Default</option>
          {levels.map((l) => (
            <option key={l} value={l}>{l}</option>
          ))}
          {effortMissing && <option value={effort}>{list ? `${effort} — not supported` : effort}</option>}
        </select>
      </Field>
      {note && (
        <span role="status" style={{ fontSize: "var(--ts-xs)", color: "var(--warn)" }}>{note}</span>
      )}
      <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)" }}>
        <Button size="sm" aria-label="refresh model list" disabled={refreshing} onClick={() => void refresh()}>
          {refreshing ? "Refreshing…" : "Refresh"}
        </Button>
        {list && <span style={{ fontSize: "var(--ts-xs)", color: "var(--text-3)" }}>{sourceLabel(list.source)}</span>}
      </div>
    </>
  );
}

const GRANTS: { grant: SimpleGrant; label: string }[] = [
  { grant: "bash", label: "Bash (all commands)" },
  { grant: "agent", label: "Agent" },
  { grant: "web-fetch", label: "WebFetch" },
  { grant: "web-search", label: "WebSearch" },
  { grant: "remote-git", label: "Remote git" },
];

/// The grant checkboxes and the Bash pattern list. Warns when Remote git is
/// granted on a model the model list says has no auto mode.
function ScopeGrants({ draft, team, onChange }: { draft: DraftPipeline; team: DraftTeam; onChange: (d: DraftPipeline) => void }) {
  const { list } = useModelList();
  const id = team.id;
  const grants = team.scope.grants;
  const model = findModel(list, team.runner.model);
  const remoteWithoutAuto = grants.includes("remote-git") && model != null && !model.supports_auto_mode;
  return (
    <>
      <fieldset style={{ border: 0, padding: 0, margin: 0, display: "grid", gap: "var(--sp-1)" }}>
        <TipLegend text={HELP.grants}>Grants</TipLegend>
        {GRANTS.map(({ grant, label }) => (
          <label key={grant} style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)", fontSize: "var(--ts-sm)", color: "var(--text-2)" }}>
            <input
              type="checkbox"
              aria-label={`${label} for ${id}`}
              checked={grants.includes(grant)}
              onChange={(e) => onChange(setTeamGrant(draft, id, grant, e.target.checked))}
            />
            {label}
          </label>
        ))}
        {remoteWithoutAuto && (
          <span role="status" style={{ fontSize: "var(--ts-xs)", color: "var(--warn)" }}>
            Remote git needs a model with auto mode. {team.runner.model} has none, so the run will not start.
          </span>
        )}
      </fieldset>
      <Field label={<LabelTip text={HELP.bashPatterns}>Bash patterns (comma-separated)</LabelTip>}>
        <input
          aria-label={`bash patterns for ${id}`}
          value={bashPatterns(team.scope).join(", ")}
          placeholder="e.g. git diff:*, npm test"
          onChange={(e) => onChange(setTeamBashPatterns(draft, id, e.target.value))}
          style={inp}
        />
      </Field>
    </>
  );
}

/// The plugins a team can declare: the installed ones, plus any the team
/// declares that are not installed (marked, so the problem is visible).
function ScopePlugins({ draft, team, onChange }: { draft: DraftPipeline; team: DraftTeam; onChange: (d: DraftPipeline) => void }) {
  const [installed, setInstalled] = useState<PluginInfo[] | null>(null);
  useEffect(() => {
    let live = true;
    Promise.resolve()
      .then(() => listPlugins())
      .then((p) => { if (live) setInstalled(p ?? []); })
      .catch(() => { if (live) setInstalled([]); });
    return () => { live = false; };
  }, []);
  const id = team.id;
  const declared = team.scope.plugins;
  const names = Array.from(new Set((installed ?? []).map((p) => p.name)));
  const missing = declared.filter((d) => !names.includes(d.split("@")[0]) && !names.includes(d));
  const rows = [...names.map((n) => ({ name: n, missing: false })), ...missing.map((n) => ({ name: n, missing: installed != null }))];
  return (
    <fieldset style={{ border: 0, padding: 0, margin: 0, display: "grid", gap: "var(--sp-1)" }}>
      <TipLegend text={HELP.plugins}>Plugins</TipLegend>
      {rows.length === 0 && (
        <span style={{ fontSize: "var(--ts-xs)", color: "var(--text-3)" }}>
          {installed == null ? "Loading plugins..." : "No plugins installed."}
        </span>
      )}
      {rows.map(({ name, missing: isMissing }) => (
        <label key={name} style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)", fontSize: "var(--ts-sm)", color: "var(--text-2)" }}>
          <input
            type="checkbox"
            aria-label={`plugin ${name} for ${id}`}
            checked={declared.includes(name)}
            onChange={(e) => onChange(setTeamPlugin(draft, id, name, e.target.checked))}
          />
          {name}
          {isMissing && <span style={{ color: "var(--warn)", fontSize: "var(--ts-xs)" }}>not installed</span>}
        </label>
      ))}
    </fieldset>
  );
}

/// G7 — a Scope reads/writes field that offers the in-app FileTreePicker
/// (multi-select within the target repo, stored repo-relative) plus the
/// comma-separated text fallback. When no targetRepo is bound, only the text
/// field shows.
function ScopePathField({
  id,
  kind,
  help,
  value,
  targetRepo,
  onChange,
}: {
  id: string;
  kind: "reads" | "writes";
  help: string;
  value: string[];
  targetRepo?: string | null;
  onChange: (raw: string) => void;
}) {
  const [browsing, setBrowsing] = useState(false);
  // The picker selects absolute paths under the repo; map back to repo-relative.
  const selectedAbs = targetRepo
    ? value.map((p) => (p.startsWith("/") ? p : `${targetRepo.replace(/\/+$/, "")}/${p}`))
    : [];

  return (
    <Field label={<LabelTip text={help}>{kind === "reads" ? "Reads" : "Writes"} (comma-separated)</LabelTip>}>
      <div style={{ display: "flex", gap: "var(--sp-2)" }}>
        <input aria-label={`${kind} for ${id}`} value={value.join(", ")} onChange={(e) => onChange(e.target.value)} style={{ ...inp, flex: 1 }} />
        {targetRepo && (
          <Button size="sm" aria-label={`browse ${kind} for ${id}`} aria-expanded={browsing} onClick={() => setBrowsing((b) => !b)}>
            {browsing ? "Close" : "Browse…"}
          </Button>
        )}
      </div>
      {browsing && targetRepo && (
        <div style={{ marginTop: "var(--sp-2)" }}>
          <FileTreePicker
            root={targetRepo}
            mode="files"
            label={`${kind} picker for ${id}`}
            selected={selectedAbs}
            onChange={(abs) => onChange(abs.map((p) => toRepoRelative(p, targetRepo)).join(", "))}
          />
        </div>
      )}
    </Field>
  );
}

function TeamEditor({ draft, id, onChange, skills, targetRepo, sessionId }: { draft: DraftPipeline; id: string; onChange: (d: DraftPipeline) => void; skills: SkillEntry[]; targetRepo?: string | null; sessionId?: string }) {
  const t = draft.teams.find((x) => x.id === id) as DraftTeam;
  // G5 — per-node "regenerate prompt": busy + error state for the one team. Hooks
  // must run unconditionally, so they sit above the early return.
  const [regenBusy, setRegenBusy] = useState(false);
  const [regenError, setRegenError] = useState<string | null>(null);
  if (!t) return null;
  const workers = t.workers;

  async function regenerate() {
    if (!sessionId) return;
    setRegenBusy(true);
    setRegenError(null);
    try {
      const next = await regenerateTeamPrompt(sessionId, draft, id);
      if (next === null) {
        setRegenError("No prompt was generated. Try again.");
      } else {
        onChange(setPromptBody(draft, id, next));
      }
    } catch (e) {
      setRegenError(e instanceof Error ? e.message : String(e));
    } finally {
      setRegenBusy(false);
    }
  }

  return (
    <div style={{ display: "grid", gap: "var(--sp-4)" }}>
      <Field label="Name">
        <input aria-label={`name for ${id}`} value={t.name} onChange={(e) => onChange(renameTeam(draft, id, e.target.value))} style={inp} />
      </Field>

      <Field label={
        <span style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: "var(--sp-3)" }}>
          <span>Prompt</span>
          {/* G5 — regenerate this one team's prompt via the Step::Prompts seam. Hidden
              when no Design Session session is available. */}
          {sessionId && (
            <Button
              size="sm"
              variant="ghost"
              aria-label={`regenerate prompt for ${id}`}
              aria-busy={regenBusy}
              disabled={regenBusy}
              onClick={regenerate}
            >
              {regenBusy ? "Regenerating…" : "Regenerate"}
            </Button>
          )}
        </span>
      }>
        <SkillAutocomplete
          aria-label={`prompt for ${id}`}
          value={t.prompt_body}
          onChange={(v, picked) => {
            const next = setPromptBody(draft, id, v);
            onChange(picked?.namespace ? addTeamPlugin(next, id, picked.namespace) : next);
          }}
          skills={skills}
          rows={6}
        />
        <div style={{ marginTop: "var(--sp-1)", fontSize: "var(--ts-xs)", color: "var(--text-3)" }}>
          Type <code style={{ fontFamily: "var(--font-mono)" }}>/</code> to insert an installed skill or command.
        </div>
        {regenError && (
          <div role="alert" style={{ marginTop: "var(--sp-1)", fontSize: "var(--ts-xs)", color: "var(--danger)" }}>
            {regenError}
          </div>
        )}
      </Field>

      <fieldset style={group}>
        <TipLegend text={HELP.role}>Role</TipLegend>
        <select aria-label={`role for ${id}`} value={t.role ?? "producer"} onChange={(e) => onChange(setTeamRole(draft, id, e.target.value as "producer" | "reviewer"))} style={inp}>
          <option value="producer">producer (hand-off)</option>
          <option value="reviewer">reviewer (approve · revise · reject)</option>
        </select>
      </fieldset>

      <fieldset style={group}>
        <TipLegend text={HELP.runner}>Runner</TipLegend>
        <div style={{ display: "grid", gap: "var(--sp-3)" }}>
          {/* Keyed by team so the snap note belongs to the team it was about. */}
          <RunnerPickers key={t.id} draft={draft} team={t} onChange={onChange} />
          <Field label="API-key env var (name only)">
            <input
              aria-label={`api key env for ${id}`}
              value={t.runner.api_key_env ?? ""}
              placeholder="e.g. ANTHROPIC_API_KEY"
              onChange={(e) =>
                onChange({ ...draft, teams: draft.teams.map((x) => (x.id === id ? { ...x, runner: { ...x.runner, api_key_env: e.target.value || null } } : x)) })
              }
              style={inp}
            />
          </Field>
        </div>
      </fieldset>

      <fieldset style={group}>
        <legend style={legend}>Scope</legend>
        <div style={{ display: "grid", gap: "var(--sp-3)" }}>
          <ScopePathField
            id={id}
            kind="reads"
            help={HELP.reads}
            value={t.scope.reads}
            targetRepo={targetRepo}
            onChange={(e) => onChange(setTeamReads(draft, id, e))}
          />
          <ScopePathField
            id={id}
            kind="writes"
            help={HELP.writes}
            value={t.scope.writes}
            targetRepo={targetRepo}
            onChange={(e) => onChange(setTeamWrites(draft, id, e))}
          />
          <ScopeGrants key={`grants-${t.id}`} draft={draft} team={t} onChange={onChange} />
          <ScopePlugins key={`plugins-${t.id}`} draft={draft} team={t} onChange={onChange} />
        </div>
      </fieldset>

      <fieldset style={group}>
        <TipLegend text={HELP.scale}>Scale (min · max)</TipLegend>
        <div style={{ display: "flex", gap: "var(--sp-3)" }}>
          <Field label="Min">
            <input type="number" min={1} aria-label={`scale min for ${id}`} value={workers.min} onChange={(e) => onChange(setTeamWorkers(draft, id, { min: Number(e.target.value) || 1, max: workers.max }))} style={inp} />
          </Field>
          <Field label="Max">
            <input type="number" min={1} aria-label={`scale max for ${id}`} value={workers.max} onChange={(e) => onChange(setTeamWorkers(draft, id, { min: workers.min, max: Number(e.target.value) || 1 }))} style={inp} />
          </Field>
        </div>
      </fieldset>

      <fieldset style={group}>
        <TipLegend text={HELP.store}>Store capacity</TipLegend>
        <Field label="WIP limit">
          <input type="number" min={1} aria-label={`store capacity for ${id}`} value={t.store?.capacity ?? 8} onChange={(e) => onChange(setTeamStoreCapacity(draft, id, Number(e.target.value) || 1))} style={inp} />
        </Field>
      </fieldset>
    </div>
  );
}

function nodeOptions(draft: DraftPipeline, excludeId?: string) {
  return [
    ...draft.teams.map((t) => ({ id: t.id, name: t.name })),
    ...draft.gates.map((g) => ({ id: g.id, name: g.label || g.id })),
    ...draft.joins.map((j) => ({ id: j.id, name: j.id })),
    ...draft.escalations.map((e) => ({ id: e.id, name: e.id })),
  ].filter((o) => o.id !== excludeId);
}

function GateEditor({ draft, id, onChange }: { draft: DraftPipeline; id: string; onChange: (d: DraftPipeline) => void }) {
  const g = draft.gates.find((x) => x.id === id);
  if (!g) return null;
  return (
    <div style={{ display: "grid", gap: "var(--sp-4)" }}>
      <Field label="Label">
        <input aria-label={`label for ${id}`} value={g.label} onChange={(e) => onChange({ ...draft, gates: draft.gates.map((x) => (x.id === id ? { ...x, label: e.target.value } : x)) })} style={inp} />
      </Field>
      <Field label="Downstream">
        <select aria-label={`downstream for ${id}`} value={g.downstream} onChange={(e) => onChange({ ...draft, gates: draft.gates.map((x) => (x.id === id ? { ...x, downstream: e.target.value } : x)) })} style={inp}>
          <option value="">(none)</option>
          {nodeOptions(draft, id).map((o) => (
            <option key={o.id} value={o.id}>{o.name} ({o.id})</option>
          ))}
        </select>
      </Field>
    </div>
  );
}

function ForkEditor({ draft, id }: { draft: DraftPipeline; id: string }) {
  const f = draft.forks.find((x) => x.id === id);
  if (!f) return null;
  return (
    <div style={{ display: "grid", gap: "var(--sp-3)" }}>
      <div style={{ fontSize: "var(--ts-sm)", color: "var(--text-3)" }}>Lanes</div>
      {f.lanes.length === 0 ? (
        <p style={{ color: "var(--text-3)", fontSize: "var(--ts-base)" }}>No lanes yet. Draw an edge from this fork to each lane node.</p>
      ) : (
        <ul aria-label={`lanes for ${id}`} style={{ listStyle: "none", margin: 0, padding: 0, display: "grid", gap: "var(--sp-2)" }}>
          {f.lanes.map((l) => (
            <li key={l} style={chip}>{l}</li>
          ))}
        </ul>
      )}
    </div>
  );
}

function JoinEditor({ draft, id, onChange }: { draft: DraftPipeline; id: string; onChange: (d: DraftPipeline) => void }) {
  const j = draft.joins.find((x) => x.id === id);
  // Hooks run unconditionally, so the help-text ids sit above the early return.
  const quorumRangeId = useId();
  const cancelHintId = useId();
  if (!j) return null;
  const m = j.waits_for.length;
  // DD7: a set quorum governs success and the runtime ignores cancel_on_reject.
  const quorumSet = j.quorum != null;

  return (
    <div style={{ display: "grid", gap: "var(--sp-4)" }}>
      <fieldset style={group}>
        <legend style={legend}>Lanes</legend>
        {m === 0 ? (
          <p style={{ color: "var(--text-3)", fontSize: "var(--ts-base)" }}>This join waits for no lanes yet.</p>
        ) : (
          <ul aria-label={`waits for lanes ${id}`} style={{ listStyle: "none", margin: 0, padding: 0, display: "grid", gap: "var(--sp-2)" }}>
            {j.waits_for.map((w) => (
              <li key={w} style={chip}>{w}</li>
            ))}
          </ul>
        )}
      </fieldset>

      <fieldset style={group}>
        <TipLegend text={HELP.quorum}>Quorum (N of M)</TipLegend>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)" }}>
          <input
            type="number"
            min={1}
            max={Math.max(1, m)}
            aria-label={`quorum for ${id}`}
            aria-describedby={quorumRangeId}
            value={j.quorum ?? ""}
            placeholder={`all ${m}`}
            onChange={(e) => {
              const v = e.target.value === "" ? undefined : Math.max(1, Number(e.target.value) || 1);
              onChange(setJoinQuorum(draft, id, v));
            }}
            style={{ ...inp, width: 80 }}
          />
          <span id={quorumRangeId} style={{ color: "var(--text-3)", fontSize: "var(--ts-sm)" }}>of {m} lanes (blank = all must approve)</span>
        </div>
      </fieldset>

      <fieldset style={group}>
        <TipLegend text={HELP.cancel}>Early-cancel on reject</TipLegend>
        <label style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)", fontSize: "var(--ts-base)", color: quorumSet ? "var(--text-4)" : "var(--text-2)", cursor: quorumSet ? "not-allowed" : "default" }}>
          <input
            type="checkbox"
            aria-label={`early cancel for ${id}`}
            aria-describedby={quorumSet ? cancelHintId : undefined}
            checked={!!j.cancel_on_reject}
            disabled={quorumSet}
            onChange={(e) => onChange(setJoinCancelOnReject(draft, id, e.target.checked))}
          />
          Cancel outstanding lanes the moment one fails
        </label>
        {quorumSet && (
          <p id={cancelHintId} style={{ color: "var(--text-3)", fontSize: "var(--ts-sm)", marginTop: "var(--sp-1)" }}>Ignored while a quorum is set.</p>
        )}
      </fieldset>
    </div>
  );
}

const body: CSSProperties = { padding: "var(--sp-8) var(--sp-7)", overflowY: "auto", height: "100%" };
const lbl: CSSProperties = { display: "block", fontSize: "var(--ts-sm)", color: "var(--text-3)" };
const inp: CSSProperties = { width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", fontFamily: "inherit", fontSize: "var(--ts-base)" };
const group: CSSProperties = { border: "1px solid var(--border)", borderRadius: "var(--r-sm)", padding: "var(--sp-3)", margin: 0 };
const legend: CSSProperties = { fontSize: "var(--ts-sm)", color: "var(--text-2)", padding: "0 var(--sp-2)" };
const chip: CSSProperties = { background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: "var(--r-sm)", padding: "var(--sp-1) var(--sp-2)", fontSize: "var(--ts-base)", color: "var(--text-2)", fontFamily: "var(--font-mono)" };
