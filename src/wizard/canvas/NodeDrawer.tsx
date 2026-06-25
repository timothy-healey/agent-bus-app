import type React from "react";
import type { CSSProperties } from "react";
import type { DraftPipeline, DraftTeam, EffortMode } from "../../ipc/pipeline";
import type { SkillEntry } from "../../ipc/skills";
import { Drawer } from "../../components/ui/Drawer";
import { Button } from "../../components/ui/Button";
import { SkillAutocomplete } from "./SkillAutocomplete";
import {
  renameTeam,
  setPromptBody,
  setTeamModel,
  setTeamEffort,
  setTeamTools,
  setTeamReads,
  setTeamWrites,
  setTeamRole,
  setTeamStoreCapacity,
  setTeamWorkers,
} from "../draft";

/// The node drawer (vet F1): a generic right-side container whose CONTENTS are
/// chosen by the selected node's kind. Header = "kind · id". Team → full config
/// (name, prompt, Runner, Scope, Role, Scale, Store). Gate → label / downstream.
/// Join → Lanes / Quorum / Early-cancel. Fork → lanes. All edits go through the
/// pure draft mutators so DraftPipeline stays the single source of truth.

const EFFORT_PRESETS: EffortMode["mode"][] = ["off", "standard", "extended-low", "extended-high", "custom"];

function effortFromSelect(mode: EffortMode["mode"], currentBudget: number): EffortMode {
  return mode === "custom" ? { mode: "custom", budget_tokens: currentBudget } : { mode };
}

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
}

type Kind = "team" | "gate" | "fork" | "join" | "escalation" | "unknown";

function kindOf(draft: DraftPipeline, id: string): Kind {
  if (draft.teams.some((t) => t.id === id)) return "team";
  if (draft.gates.some((g) => g.id === id)) return "gate";
  if (draft.forks.some((f) => f.id === id)) return "fork";
  if (draft.joins.some((j) => j.id === id)) return "join";
  if (draft.escalations.some((e) => e.id === id)) return "escalation";
  return "unknown";
}

export function NodeDrawer({ draft, selectedId, onChange, onClose, skills = [], onDelete }: NodeDrawerProps) {
  const open = selectedId != null;
  const kind = selectedId ? kindOf(draft, selectedId) : "unknown";

  return (
    <Drawer open={open} onClose={onClose} label={selectedId ? `${kind} · ${selectedId}` : "node"}>
      {selectedId && (
        <div style={body}>
          <header style={{ marginBottom: "var(--sp-5)", display: "flex", alignItems: "center", justifyContent: "space-between", gap: "var(--sp-3)" }}>
            <div style={{ fontSize: "var(--ts-sm)", color: "var(--text-3)", textTransform: "capitalize", letterSpacing: "0.04em" }}>
              {kind} · <span style={{ color: "var(--text-2)" }}>{selectedId}</span>
            </div>
            {onDelete && (
              <Button variant="danger" size="sm" onClick={() => onDelete(selectedId)} aria-label={`delete ${selectedId}`}>
                Delete
              </Button>
            )}
          </header>
          {kind === "team" && <TeamEditor draft={draft} id={selectedId} onChange={onChange} skills={skills} />}
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

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label style={lbl}>
      <span style={{ display: "block", marginBottom: "var(--sp-1)" }}>{label}</span>
      {children}
    </label>
  );
}

function TeamEditor({ draft, id, onChange, skills }: { draft: DraftPipeline; id: string; onChange: (d: DraftPipeline) => void; skills: SkillEntry[] }) {
  const t = draft.teams.find((x) => x.id === id) as DraftTeam;
  if (!t) return null;
  const effort = t.runner.effort;
  const workers = t.workers;

  return (
    <div style={{ display: "grid", gap: "var(--sp-4)" }}>
      <Field label="Name">
        <input aria-label={`name for ${id}`} value={t.name} onChange={(e) => onChange(renameTeam(draft, id, e.target.value))} style={inp} />
      </Field>

      <Field label="Prompt">
        <SkillAutocomplete
          aria-label={`prompt for ${id}`}
          value={t.prompt_body}
          onChange={(v) => onChange(setPromptBody(draft, id, v))}
          skills={skills}
          rows={6}
        />
        <div style={{ marginTop: "var(--sp-1)", fontSize: "var(--ts-xs)", color: "var(--text-3)" }}>
          Type <code style={{ fontFamily: "var(--font-mono)" }}>/</code> to insert an installed skill or command.
        </div>
      </Field>

      <fieldset style={group}>
        <legend style={legend}>Role</legend>
        <select aria-label={`role for ${id}`} value={t.role ?? "producer"} onChange={(e) => onChange(setTeamRole(draft, id, e.target.value as "producer" | "reviewer"))} style={inp}>
          <option value="producer">producer (hand-off)</option>
          <option value="reviewer">reviewer (approve · revise · reject)</option>
        </select>
      </fieldset>

      <fieldset style={group}>
        <legend style={legend}>Runner</legend>
        <div style={{ display: "grid", gap: "var(--sp-3)" }}>
          <Field label="Model">
            <input aria-label={`model for ${id}`} value={t.runner.model} onChange={(e) => onChange(setTeamModel(draft, id, e.target.value))} style={inp} />
          </Field>
          <Field label="Effort">
            <select
              aria-label={`effort for ${id}`}
              value={effort.mode}
              onChange={(e) =>
                onChange(
                  setTeamEffort(
                    draft,
                    id,
                    effortFromSelect(e.target.value as EffortMode["mode"], effort.mode === "custom" ? effort.budget_tokens : 16000),
                  ),
                )
              }
              style={inp}
            >
              {EFFORT_PRESETS.map((m) => (
                <option key={m} value={m}>{m}</option>
              ))}
            </select>
          </Field>
          {effort.mode === "custom" && (
            <Field label="Budget (tokens)">
              <input type="number" aria-label={`budget for ${id}`} value={effort.budget_tokens} onChange={(e) => onChange(setTeamEffort(draft, id, { mode: "custom", budget_tokens: Number(e.target.value) || 0 }))} style={inp} />
            </Field>
          )}
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
          <Field label="Reads (comma-separated)">
            <input aria-label={`reads for ${id}`} value={t.scope.reads.join(", ")} onChange={(e) => onChange(setTeamReads(draft, id, e.target.value))} style={inp} />
          </Field>
          <Field label="Writes (comma-separated)">
            <input aria-label={`writes for ${id}`} value={t.scope.writes.join(", ")} onChange={(e) => onChange(setTeamWrites(draft, id, e.target.value))} style={inp} />
          </Field>
          <Field label="Tools (comma-separated)">
            <input aria-label={`tools for ${id}`} value={t.scope.tools.join(", ")} onChange={(e) => onChange(setTeamTools(draft, id, e.target.value))} style={inp} />
          </Field>
        </div>
      </fieldset>

      <fieldset style={group}>
        <legend style={legend}>Scale (min · max)</legend>
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
        <legend style={legend}>Store capacity</legend>
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
  if (!j) return null;
  const setJoin = (patch: Partial<typeof j>) => onChange({ ...draft, joins: draft.joins.map((x) => (x.id === id ? { ...x, ...patch } : x)) });
  const m = j.waits_for.length;

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
        <legend style={legend}>Quorum (N of M)</legend>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)" }}>
          <input
            type="number"
            min={1}
            max={Math.max(1, m)}
            aria-label={`quorum for ${id}`}
            value={j.quorum ?? ""}
            placeholder={`all ${m}`}
            onChange={(e) => {
              const v = e.target.value === "" ? undefined : Math.max(1, Number(e.target.value) || 1);
              setJoin({ quorum: v });
            }}
            style={{ ...inp, width: 80 }}
          />
          <span style={{ color: "var(--text-3)", fontSize: "var(--ts-sm)" }}>of {m} lanes (blank = all must approve)</span>
        </div>
      </fieldset>

      <fieldset style={group}>
        <legend style={legend}>Early-cancel on reject</legend>
        <label style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)", fontSize: "var(--ts-base)", color: "var(--text-2)" }}>
          <input
            type="checkbox"
            aria-label={`early cancel for ${id}`}
            checked={!!j.cancel_on_reject}
            disabled={j.quorum != null}
            onChange={(e) => setJoin({ cancel_on_reject: e.target.checked })}
          />
          Cancel outstanding lanes the moment one fails
        </label>
        {j.quorum != null && (
          <p style={{ color: "var(--text-3)", fontSize: "var(--ts-sm)", marginTop: "var(--sp-1)" }}>Ignored while a quorum is set.</p>
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
