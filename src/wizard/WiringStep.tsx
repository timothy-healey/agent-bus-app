import { useState } from "react";
import type React from "react";
import type { DraftPipeline, Pipeline } from "../ipc/pipeline";
import { PipelineView } from "../components/PipelineView";
import { addGate, setTeamApprove } from "./draft";

/// Adapt a DraftPipeline to the Pipeline shape the read-only viewer expects:
/// inline prompt bodies become placeholder paths; gates carry through (W3).
export function draftToPipeline(d: DraftPipeline): Pipeline {
  return {
    id: d.id || "(draft)",
    name: d.name || "(unnamed)",
    description: d.description,
    schema_version: d.schema_version,
    teams: d.teams.map((t) => ({ ...t, prompt: `prompts/${t.id}.md` })),
    gates: d.gates,
    escalations: d.escalations,
    forks: d.forks,
    joins: d.joins,
  };
}

interface WiringStepProps {
  draft: DraftPipeline;
  onChange: (d: DraftPipeline) => void;
}

/// Step 4 draft view: the fork/join/gate flow rendered through PipelineView, plus
/// an add-human-gate affordance (DD1 Option C). Adding a gate = push the gate node
/// AND repoint the chosen upstream team's on_approve at it (DD2: upstream implied).
export function WiringStep({ draft, onChange }: WiringStepProps) {
  const [gid, setGid] = useState("");
  const [label, setLabel] = useState("");
  const [upstream, setUpstream] = useState("");
  const [downstream, setDownstream] = useState("");

  const canAdd = gid.trim() && label.trim() && upstream && downstream;
  const nodeOptions = [
    ...draft.teams.map((t) => ({ id: t.id, name: t.name })),
    ...draft.escalations.map((e) => ({ id: e.id, name: e.id })),
    ...draft.joins.map((j) => ({ id: j.id, name: j.id })),
  ];

  function add() {
    if (!canAdd) return;
    let next = addGate(draft, gid.trim(), label.trim(), downstream);
    next = setTeamApprove(next, upstream, gid.trim());
    onChange(next);
    setGid("");
    setLabel("");
    setUpstream("");
    setDownstream("");
  }

  return (
    <div>
      <div style={{ display: "grid", gap: 6, marginBottom: "var(--sp-3)", padding: "var(--sp-2)", border: "1px solid var(--border)", borderRadius: "var(--r-sm)" }}>
        <div style={{ fontSize: 11, color: "var(--text-3)" }}>Add a human-review gate</div>
        <input aria-label="gate id" placeholder="gate id (e.g. gate-2)" value={gid} onChange={(e) => setGid(e.target.value)} style={inp} />
        <input aria-label="gate label" placeholder="label (e.g. Plan review)" value={label} onChange={(e) => setLabel(e.target.value)} style={inp} />
        <select aria-label="gate upstream" value={upstream} onChange={(e) => setUpstream(e.target.value)} style={inp}>
          <option value="">upstream team (its on_approve routes here)…</option>
          {draft.teams.map((t) => (
            <option key={t.id} value={t.id}>{t.name} ({t.id})</option>
          ))}
        </select>
        <select aria-label="gate downstream" value={downstream} onChange={(e) => setDownstream(e.target.value)} style={inp}>
          <option value="">downstream node…</option>
          {nodeOptions.map((n) => (
            <option key={n.id} value={n.id}>{n.name} ({n.id})</option>
          ))}
        </select>
        <button onClick={add} disabled={!canAdd} aria-label="add gate">Add gate</button>
      </div>
      <PipelineView pipeline={draftToPipeline(draft)} />
    </div>
  );
}

const inp: React.CSSProperties = { width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)" };
