import { type ReactNode, useState } from "react";
import { designSessionTurn, type DraftPipeline, type Step } from "../ipc/pipeline";

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
/// edits (via renderDraft's onChange) mutate the draft so the next turn sends it.
export function ChatDraftPanel({ sessionId, step, draft, onDraftChange, renderDraft }: ChatDraftPanelProps) {
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  async function send() {
    const v = value.trim();
    if (!v || busy) return;
    setBusy(true);
    setMsgs((m) => [...m, { role: "you", text: v }]);
    setValue("");
    try {
      const out = await designSessionTurn(sessionId, step, draft, v);
      setMsgs((m) => [...m, { role: "claude", text: out.reply_text }]);
      onDraftChange(out.updated_draft);
    } catch (e) {
      setMsgs((m) => [...m, { role: "claude", text: `[error] ${e instanceof Error ? e.message : String(e)}` }]);
    } finally {
      setBusy(false);
    }
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
      <div style={{ flex: 1, overflowY: "auto", minWidth: 280 }}>{renderDraft(draft, onDraftChange)}</div>
    </div>
  );
}
