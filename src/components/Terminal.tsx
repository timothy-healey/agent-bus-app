import { useState } from "react";
import type { Turn } from "../ipc/terminal";
import { toolChip } from "../lib/toolChip";

interface TerminalProps {
  turns: Turn[];
  contextLine: string;
  onSend: (input: string) => void;
}

/// The always-present god terminal docked at the bottom. Collapsible to a thin
/// bar (DESIGN.md). Max height 280px. Tool calls render as inline chips.
export function Terminal({ turns, contextLine, onSend }: TerminalProps) {
  const [value, setValue] = useState("");
  const [collapsed, setCollapsed] = useState(false);

  function submit() {
    const v = value.trim();
    if (!v) return;
    onSend(v);
    setValue("");
  }

  return (
    <div
      style={{
        background: "var(--bg-2)",
        borderTop: "1px solid var(--border)",
        fontFamily: "var(--font-mono, ui-monospace, monospace)",
        fontSize: "var(--ts-base, 12.5px)",
        color: "var(--text-2)",
        display: "flex",
        flexDirection: "column",
        maxHeight: collapsed ? 28 : 280,
      }}
    >
      <div
        style={{
          height: 28,
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "0 12px",
          borderBottom: collapsed ? "none" : "1px solid var(--border)",
          flexShrink: 0,
        }}
      >
        <span style={{ color: "var(--accent)" }}>●</span>
        <span style={{ color: "var(--text-2)" }}>claude</span>
        <span style={{ color: "var(--text-3)", marginLeft: 8 }}>context: {contextLine}</span>
        <button
          aria-label={collapsed ? "expand terminal" : "collapse terminal"}
          onClick={() => setCollapsed((c) => !c)}
          style={{
            marginLeft: "auto",
            background: "transparent",
            border: "none",
            color: "var(--text-3)",
            cursor: "pointer",
          }}
        >
          {collapsed ? "▲" : "▼"}
        </button>
      </div>

      {!collapsed && (
        <>
          <div style={{ flex: 1, overflowY: "auto", padding: "8px 12px" }}>
            {turns.map((t, i) => (
              <div key={i} style={{ marginBottom: 10 }}>
                <div style={{ color: "var(--text-3)", marginBottom: 2 }}>
                  {t.role === "user" ? "you" : "claude"}
                </div>
                <div style={{ color: "var(--text)" }}>{t.text}</div>
                {t.tool_calls.length > 0 && (
                  <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 4 }}>
                    {t.tool_calls.map((tc, j) => {
                      const chip = toolChip(tc);
                      return (
                        <span
                          key={j}
                          style={{
                            background: "var(--surface)",
                            border: "1px solid var(--border)",
                            borderRadius: 5,
                            padding: "3px 10px",
                          }}
                        >
                          <span style={{ color: "var(--accent)" }}>→</span> {chip.label}{" "}
                          <span style={{ color: chip.ok ? "var(--running)" : "var(--text-3)" }}>
                            {chip.glyph}
                          </span>
                        </span>
                      );
                    })}
                  </div>
                )}
              </div>
            ))}
          </div>

          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              padding: "6px 12px",
              borderTop: "1px solid var(--border)",
              flexShrink: 0,
            }}
          >
            <span style={{ color: "var(--accent)" }}>›</span>
            <input
              value={value}
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") submit(); }}
              placeholder="ask, inject, approve, brake, scale, anything…"
              style={{
                flex: 1,
                background: "transparent",
                border: "none",
                outline: "none",
                color: "var(--text)",
                fontFamily: "inherit",
                fontSize: "inherit",
              }}
            />
            <span style={{ color: "var(--text-3)", fontSize: "var(--ts-sm, 11px)" }}>↑ history</span>
            <span style={{ color: "var(--text-3)", fontSize: "var(--ts-sm, 11px)" }}>⌘K commands</span>
          </div>
        </>
      )}
    </div>
  );
}
