import type { ToolCall } from "../ipc/terminal";

export interface ChipView {
  label: string;
  glyph: "✓" | "⏳" | "✗";
  ok: boolean;
}

/// Render a ToolCall as the inline chip the mockup shows: "→ <name> <arg> <glyph>".
/// The arg shown is the first scalar value in the request args (e.g. the topic or
/// task_id); falls back to just the tool name. Pure so it is unit-tested.
export function toolChip(tc: ToolCall): ChipView {
  const args = (tc.request.args ?? {}) as Record<string, unknown>;
  const firstScalar = Object.values(args).find(
    (v) => typeof v === "string" || typeof v === "number",
  );
  const label = firstScalar != null
    ? `${tc.request.tool_name} ${String(firstScalar)}`
    : tc.request.tool_name;

  let glyph: ChipView["glyph"] = "⏳";
  let ok = false;
  if (tc.result?.status === "ok") {
    glyph = "✓";
    ok = true;
  } else if (tc.result?.status === "err") {
    glyph = "✗";
  }
  return { label, glyph, ok };
}
