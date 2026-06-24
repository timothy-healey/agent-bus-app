import { useState } from "react";
import type React from "react";
import type { DraftPipeline, Pipeline } from "../ipc/pipeline";
import { PipelineView } from "../components/PipelineView";
import { addForkJoin, addGate, setTeamApprove } from "./draft";
import { Button } from "../components/ui/Button";

/// Adapt a DraftPipeline to the Pipeline shape the read-only viewer expects:
/// inline prompt bodies become placeholder paths; gates carry through (W3).
export function draftToPipeline(d: DraftPipeline): Pipeline {
  return {
    id: d.id || "(draft)",
    name: d.name || "(unnamed)",
    description: d.description,
    schema_version: d.schema_version,
    teams: d.teams.map((t) => ({ ...t, prompt: `prompts/${t.id}.md`, role: "producer" as const, store: { capacity: 8 } })),
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

  const [fid, setFid] = useState("");
  const [lane1, setLane1] = useState("");
  const [lane2, setLane2] = useState("");
  const [forkDown, setForkDown] = useState("");

  const canAdd = gid.trim() && label.trim() && upstream && downstream;
  const canAddFork = fid.trim() && lane1 && lane2 && lane1 !== lane2 && forkDown;
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

  function addFork() {
    if (!canAddFork) return;
    const suffix = fid.trim();
    const next = addForkJoin(draft, `fork-${suffix}`, `join-${suffix}`, [lane1, lane2], forkDown);
    onChange(next);
    setFid("");
    setLane1("");
    setLane2("");
    setForkDown("");
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
        <Button onClick={add} disabled={!canAdd} aria-label="add gate">Add gate</Button>
      </div>
      <div style={{ display: "grid", gap: 6, marginBottom: "var(--sp-3)", padding: "var(--sp-2)", border: "1px solid var(--border)", borderRadius: "var(--r-sm)" }}>
        <div style={{ fontSize: 11, color: "var(--text-3)" }}>Add a parallel fork (creates a paired fork + join)</div>
        <input aria-label="fork id" placeholder="fork id (e.g. 1 → fork-1 / join-1)" value={fid} onChange={(e) => setFid(e.target.value)} style={inp} />
        <select aria-label="fork lane 1" value={lane1} onChange={(e) => setLane1(e.target.value)} style={inp}>
          <option value="">lane 1 team…</option>
          {draft.teams.map((t) => (
            <option key={t.id} value={t.id}>{t.name} ({t.id})</option>
          ))}
        </select>
        <select aria-label="fork lane 2" value={lane2} onChange={(e) => setLane2(e.target.value)} style={inp}>
          <option value="">lane 2 team…</option>
          {draft.teams.map((t) => (
            <option key={t.id} value={t.id}>{t.name} ({t.id})</option>
          ))}
        </select>
        <select aria-label="fork downstream" value={forkDown} onChange={(e) => setForkDown(e.target.value)} style={inp}>
          <option value="">downstream node (after join)…</option>
          {nodeOptions.map((n) => (
            <option key={n.id} value={n.id}>{n.name} ({n.id})</option>
          ))}
        </select>
        <Button onClick={addFork} disabled={!canAddFork} aria-label="add fork">Add fork</Button>
      </div>
      <PipelineView pipeline={draftToPipeline(draft)} />
    </div>
  );
}

const inp: React.CSSProperties = { width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", fontFamily: "inherit", fontSize: "var(--ts-base)" };
