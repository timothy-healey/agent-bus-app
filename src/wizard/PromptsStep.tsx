import type { DraftPipeline } from "../ipc/pipeline";
import { setPromptBody } from "./draft";

interface PromptsStepProps {
  draft: DraftPipeline;
  onChange: (d: DraftPipeline) => void;
}

/// Step 3 draft view: each team's responsibility prompt text, editable.
export function PromptsStep({ draft, onChange }: PromptsStepProps) {
  return (
    <div>
      {draft.teams.map((t) => (
        <div key={t.id} style={{ marginBottom: "var(--sp-3)" }}>
          <div style={{ color: "var(--text-2)", fontSize: 12, marginBottom: 4 }}>{t.name} · {t.id}</div>
          <textarea
            aria-label={`prompt for ${t.id}`}
            value={t.prompt_body}
            onChange={(e) => onChange(setPromptBody(draft, t.id, e.target.value))}
            rows={5}
            style={{ width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)", fontFamily: "inherit", fontSize: 12 }}
          />
        </div>
      ))}
    </div>
  );
}
