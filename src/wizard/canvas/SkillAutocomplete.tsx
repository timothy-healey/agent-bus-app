import type React from "react";
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
} from "react";
import type { SkillEntry } from "../../ipc/skills";
import {
  applyInsertion,
  findTrigger,
  isRecognized,
  matchEntries,
  scanTokens,
  type TriggerToken,
} from "./skillTokens";

/// A4 — discovery-backed `/`-autocomplete over a prompt textarea, plus a mirror-
/// overlay layer that tints recognized `/skill` tokens. Authoring-time only; the
/// component consumes `SkillEntry[]` and emits text via `onChange` (the existing
/// draft path). Pure logic lives in `skillTokens.ts`.

export interface SkillAutocompleteProps {
  value: string;
  onChange: (value: string) => void;
  skills: SkillEntry[];
  rows?: number;
  "aria-label"?: string;
  style?: CSSProperties;
}

type Mode =
  | { kind: "closed" }
  | { kind: "skills"; trigger: TriggerToken; active: number }
  | { kind: "verbs"; entry: SkillEntry; trigger: TriggerToken; active: number };

const MAX_ROWS = 8;

export function SkillAutocomplete({
  value,
  onChange,
  skills,
  rows = 6,
  "aria-label": ariaLabel,
  style,
}: SkillAutocompleteProps) {
  const taRef = useRef<HTMLTextAreaElement>(null);
  const overlayRef = useRef<HTMLDivElement>(null);
  const [mode, setMode] = useState<Mode>({ kind: "closed" });
  const listId = useId();

  // The filtered skill list for the current trigger query.
  const matches = useMemo(() => {
    if (mode.kind !== "skills") return [];
    return matchEntries(skills, mode.trigger.query).slice(0, MAX_ROWS);
  }, [mode, skills]);

  const close = useCallback(() => setMode({ kind: "closed" }), []);

  // Re-evaluate the trigger whenever the text or caret changes.
  const reevaluate = useCallback(
    (text: string) => {
      const ta = taRef.current;
      if (!ta) return;
      const caret = ta.selectionStart ?? text.length;
      const trigger = findTrigger(text, caret);
      if (!trigger) {
        setMode({ kind: "closed" });
        return;
      }
      setMode((prev) => {
        // Keep the verb cascade open while it is showing.
        if (prev.kind === "verbs") return { ...prev, trigger };
        return { kind: "skills", trigger, active: 0 };
      });
    },
    [],
  );

  function handleChange(next: string) {
    onChange(next);
    // Defer so selectionStart reflects the new caret.
    requestAnimationFrame(() => reevaluate(next));
  }

  function commit(entry: SkillEntry, trigger: TriggerToken) {
    // Open the verb cascade when the entry declares verbs; else insert + close.
    if (entry.verbs.length > 0) {
      setMode({ kind: "verbs", entry, trigger, active: 0 });
      return;
    }
    insert(entry, trigger);
  }

  function insert(entry: SkillEntry, trigger: TriggerToken, verb?: string) {
    const ta = taRef.current;
    const caret = ta?.selectionStart ?? value.length;
    const result = applyInsertion(value, caret, trigger, entry, verb);
    onChange(result.text);
    close();
    requestAnimationFrame(() => {
      const el = taRef.current;
      if (el) {
        el.focus();
        el.setSelectionRange(result.caret, result.caret);
      }
    });
  }

  function onKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (mode.kind === "closed") return;
    const items: number = mode.kind === "skills" ? matches.length : mode.entry.verbs.length;
    if (mode.kind === "skills" && items === 0) {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
      return;
    }
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        setMode((m) => (m.kind === "closed" ? m : { ...m, active: (m.active + 1) % items }));
        break;
      case "ArrowUp":
        e.preventDefault();
        setMode((m) => (m.kind === "closed" ? m : { ...m, active: (m.active - 1 + items) % items }));
        break;
      case "Enter":
        e.preventDefault();
        if (mode.kind === "skills") {
          const entry = matches[mode.active];
          if (entry) commit(entry, mode.trigger);
        } else {
          insert(mode.entry, mode.trigger, mode.entry.verbs[mode.active]);
        }
        break;
      case "Escape":
        e.preventDefault();
        // From the verb cascade, Esc steps back to the skill list; from the
        // skill list it dismisses.
        if (mode.kind === "verbs") {
          setMode({ kind: "skills", trigger: mode.trigger, active: 0 });
        } else {
          close();
        }
        break;
      default:
        break;
    }
  }

  // Keep the overlay scrolled in sync with the textarea.
  function syncScroll() {
    const ta = taRef.current;
    const ov = overlayRef.current;
    if (ta && ov) {
      ov.scrollTop = ta.scrollTop;
      ov.scrollLeft = ta.scrollLeft;
    }
  }

  // Recompute on outside value changes (e.g. switching nodes).
  useEffect(() => {
    syncScroll();
  }, [value]);

  const open = mode.kind !== "closed";
  // A listbox is only in the DOM when there are rows to show (the verb cascade
  // always has rows; the skill list only when there are matches).
  const hasListbox =
    (mode.kind === "skills" && matches.length > 0) || mode.kind === "verbs";
  const activeId = !hasListbox
    ? undefined
    : mode.kind === "skills"
      ? `${listId}-s-${mode.active}`
      : `${listId}-v-${mode.active}`;

  return (
    <div style={{ position: "relative", ...style }}>
      {/* Mirror-overlay highlight layer (behind the textarea). */}
      <div ref={overlayRef} aria-hidden style={overlayStyle}>
        {renderHighlighted(value, skills)}
      </div>
      <textarea
        ref={taRef}
        aria-label={ariaLabel}
        aria-expanded={open}
        aria-controls={hasListbox ? listId : undefined}
        aria-activedescendant={activeId}
        role="combobox"
        aria-autocomplete="list"
        value={value}
        rows={rows}
        onChange={(e) => handleChange(e.target.value)}
        onKeyDown={onKeyDown}
        onScroll={syncScroll}
        onClick={() => reevaluate(value)}
        onBlur={() => requestAnimationFrame(close)}
        style={textareaStyle}
        spellCheck={false}
      />

      {mode.kind === "skills" && matches.length === 0 && skills.length > 0 && (
        <div role="status" style={{ ...popoverStyle, display: "block" }}>
          <span style={emptyText}>no skill or command matches “{mode.trigger.query}”</span>
        </div>
      )}

      {mode.kind === "skills" && matches.length > 0 && (
        <ul id={listId} role="listbox" aria-label="skills" style={popoverStyle}>
          {matches.map((entry, i) => (
            <li
              key={`${entry.source}:${entry.namespace ?? ""}:${entry.name}`}
              id={`${listId}-s-${i}`}
              role="option"
              aria-selected={i === mode.active}
              // Use onMouseDown (fires before blur) so the click lands.
              onMouseDown={(e) => {
                e.preventDefault();
                commit(entry, mode.trigger);
              }}
              onMouseEnter={() => setMode((m) => (m.kind === "skills" ? { ...m, active: i } : m))}
              style={i === mode.active ? { ...rowStyle, ...rowActive } : rowStyle}
            >
              <SkillRow entry={entry} />
            </li>
          ))}
        </ul>
      )}

      {mode.kind === "verbs" && (
        <ul id={listId} role="listbox" aria-label={`verbs for ${mode.entry.name}`} style={popoverStyle}>
          {mode.entry.verbs.map((verb, i) => (
            <li
              key={verb}
              id={`${listId}-v-${i}`}
              role="option"
              aria-selected={i === mode.active}
              onMouseDown={(e) => {
                e.preventDefault();
                insert(mode.entry, mode.trigger, verb);
              }}
              onMouseEnter={() => setMode((m) => (m.kind === "verbs" ? { ...m, active: i } : m))}
              style={i === mode.active ? { ...rowStyle, ...rowActive } : rowStyle}
            >
              <span style={tokenText}>/{mode.entry.name} {verb}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function SkillRow({ entry }: { entry: SkillEntry }) {
  const form = entry.qualified && entry.namespace ? `${entry.namespace}:${entry.name}` : entry.name;
  return (
    <div style={{ display: "grid", gap: "var(--sp-1)", minWidth: 0 }}>
      <div style={{ display: "flex", alignItems: "baseline", gap: "var(--sp-2)", minWidth: 0 }}>
        <span style={tokenText}>/{form}</span>
        <span style={tag}>· {entry.kind}</span>
        <span style={tag}>· {entry.source}</span>
        {entry.verbs.length > 0 && <span style={tag}>· {entry.verbs.length} verbs ▸</span>}
      </div>
      {entry.description && <div style={descText}>{entry.description}</div>}
    </div>
  );
}

/** Render the body with recognized `/skill` tokens wrapped in a tint span. The
 *  text content is identical to the textarea so the overlay aligns char-for-char.
 *  Exported for the component test. */
export function renderHighlighted(text: string, skills: SkillEntry[]) {
  const tokens = scanTokens(text);
  const parts: React.ReactNode[] = [];
  let cursor = 0;
  tokens.forEach((tok, idx) => {
    if (tok.start > cursor) parts.push(text.slice(cursor, tok.start));
    if (isRecognized(tok.name, skills)) {
      parts.push(
        <span key={`t-${idx}`} className="abp-skill-token" data-recognized="true" style={tokenTint}>
          {tok.text}
        </span>,
      );
    } else {
      parts.push(tok.text);
    }
    cursor = tok.end;
  });
  if (cursor < text.length) parts.push(text.slice(cursor));
  // A trailing newline needs a placeholder so the overlay height matches.
  return parts.length ? parts : "";
}

// Shared metrics so the overlay text aligns exactly with the textarea text.
const sharedText: CSSProperties = {
  font: "inherit",
  fontFamily: "var(--font-mono)",
  fontSize: "var(--ts-base)",
  lineHeight: 1.5,
  padding: "var(--sp-2)",
  whiteSpace: "pre-wrap",
  wordBreak: "break-word",
  letterSpacing: "normal",
  boxSizing: "border-box",
};

const textareaStyle: CSSProperties = {
  ...sharedText,
  position: "relative",
  width: "100%",
  background: "transparent",
  border: "1px solid var(--border)",
  color: "var(--text)",
  borderRadius: "var(--r-sm)",
  resize: "vertical",
  // The caret + typed text sit above the overlay.
  caretColor: "var(--text)",
};

const overlayStyle: CSSProperties = {
  ...sharedText,
  position: "absolute",
  inset: 0,
  border: "1px solid transparent",
  color: "transparent",
  overflow: "hidden",
  pointerEvents: "none",
  zIndex: 0,
};

const tokenTint: CSSProperties = {
  borderRadius: "var(--r-xs)",
  background: "var(--accent-3)",
  boxShadow: "0 0 0 1px var(--accent-bd)",
  color: "transparent",
};

const popoverStyle: CSSProperties = {
  position: "absolute",
  top: "100%",
  left: 0,
  marginTop: "var(--sp-1)",
  zIndex: 20,
  listStyle: "none",
  margin: 0,
  padding: "var(--sp-1)",
  width: "100%",
  maxHeight: 280,
  overflowY: "auto",
  background: "var(--surface-3)",
  border: "1px solid var(--border-2)",
  borderRadius: "var(--r-sm)",
  boxShadow: "var(--shadow-popover)",
  display: "grid",
  gap: "var(--sp-1)",
};

const rowStyle: CSSProperties = {
  padding: "var(--sp-2)",
  borderRadius: "var(--r-xs)",
  border: "1px solid transparent",
  cursor: "pointer",
  fontSize: "var(--ts-sm)",
  transition: "background var(--dur-fast) var(--ease-out)",
};

// Aligns to DESIGN.md §States "selected": accent-2 surface + accent-bd border.
const rowActive: CSSProperties = {
  background: "var(--accent-2)",
  borderColor: "var(--accent-bd)",
};

const tokenText: CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: "var(--ts-sm)",
  color: "var(--text)",
};

const tag: CSSProperties = {
  fontSize: "var(--ts-xs)",
  color: "var(--text-3)",
  whiteSpace: "nowrap",
};

const emptyText: CSSProperties = {
  display: "block",
  padding: "var(--sp-2)",
  fontSize: "var(--ts-sm)",
  color: "var(--text-3)",
};

const descText: CSSProperties = {
  fontSize: "var(--ts-xs)",
  color: "var(--text-3)",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};
