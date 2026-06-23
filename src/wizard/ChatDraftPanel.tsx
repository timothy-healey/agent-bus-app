import { type ReactNode, useState } from "react";
import { bestEffortValidate, designSessionTurn, type DraftPipeline, type Step } from "../ipc/pipeline";

interface Msg {
  role: "you" | "claude";
  text: string;
}

interface ChatDraftPanelProps {
  sessionId: string;
  step: Step;
  draft: DraftPipeline;
  onDraftChange: (d: DraftPipeline) => void;
  renderDraft: (d: DraftPipeline, onChange: (d: DraftPipeline) => void) => ReactNode;
}

/// The shared two-way-bound panel (layout A): chat left, live-editable draft
/// right. A chat turn emits a slice that updates the draft + is narrated; manual
/// edits mutate the draft so the next turn sends it. W1: best-effort validation
/// issues are surfaced inline — from the turn result, and re-fetched from the
/// backend on every manual edit (the backend stays the validation authority).
export function ChatDraftPanel({ sessionId, step, draft, onDraftChange, renderDraft }: ChatDraftPanelProps) {
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [issues, setIssues] = useState<string[]>([]);

  async function send() {
    const v = value.trim();
    if (!v || busy) return;
    setBusy(true);
    setMsgs((m) => [...m, { role: "you", text: v }]);
    setValue("");
    try {
      const out = await designSessionTurn(sessionId, step, draft, v);
      setMsgs((m) => [...m, { role: "claude", text: out.reply_text }]);
      setIssues(out.issues);
      onDraftChange(out.updated_draft);
    } catch (e) {
      setMsgs((m) => [...m, { role: "claude", text: `[error] ${e instanceof Error ? e.message : String(e)}` }]);
    } finally {
      setBusy(false);
    }
  }

  // Manual edits bypass chat; re-fetch the backend's best-effort issues so the
  // banner stays live without any validation logic in the frontend (W1).
  function handleManualEdit(d: DraftPipeline) {
    onDraftChange(d);
    bestEffortValidate(d).then(setIssues).catch(() => {});
  }

  return (
    <div style={{ display: "flex", gap: "var(--sp-5)", height: "100%" }}>
      <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 280 }}>
        <div style={{ flex: 1, overflowY: "auto", border: "1px solid var(--border)", borderRadius: "var(--r-sm)", padding: "var(--sp-3)" }}>
          {msgs.map((m, i) => (
            <div key={i} style={{ marginBottom: 8 }}>
              <div style={{ color: "var(--text-3)", fontSize: 11 }}>{m.role}</div>
              <div style={{ color: "var(--text)" }}>{m.text}</div>
            </div>
          ))}
        </div>
        <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
          <input
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") send(); }}
            placeholder="refine this step…"
            style={{ flex: 1, background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)" }}
          />
          <button onClick={send} disabled={busy} aria-label="send">Send</button>
        </div>
      </div>
      <div style={{ flex: 1, overflowY: "auto", minWidth: 280 }}>
        {issues.length > 0 && (
          <div
            role="status"
            aria-label="validation issues"
            style={{ marginBottom: "var(--sp-3)", padding: "var(--sp-2)", border: "1px solid var(--accent-bd)", background: "var(--accent-2)", borderRadius: "var(--r-sm)", color: "var(--text-2)", fontSize: 11 }}
          >
            {issues.map((iss, i) => (
              <div key={i}>• {iss}</div>
            ))}
          </div>
        )}
        {renderDraft(draft, handleManualEdit)}
      </div>
    </div>
  );
}
