import { describe, expect, it } from "vitest";
import { toolChip } from "./toolChip";
import type { ToolCall } from "../ipc/terminal";

function call(name: string, args: unknown, result: ToolCall["result"]): ToolCall {
  return { request: { tool_name: name, args }, result };
}

describe("toolChip", () => {
  it("renders an ok call with the check glyph", () => {
    const c = toolChip(call("approve_gate", { task_id: "T-041" }, { status: "ok", result: {} }));
    expect(c.label).toBe("approve_gate T-041");
    expect(c.glyph).toBe("✓");
    expect(c.ok).toBe(true);
  });

  it("renders a pending (null result) call with the hourglass", () => {
    const c = toolChip(call("inject_topic", { topic: "03-x" }, null));
    expect(c.label).toBe("inject_topic 03-x");
    expect(c.glyph).toBe("⏳");
    expect(c.ok).toBe(false);
  });

  it("renders an error call distinctly", () => {
    const c = toolChip(call("inject_topic", { topic: "x" }, { status: "err", error: "boom" }));
    expect(c.glyph).toBe("✗");
    expect(c.ok).toBe(false);
  });

  it("falls back to the tool name when no scalar arg is present", () => {
    const c = toolChip(call("usage_snapshot", {}, { status: "ok", result: {} }));
    expect(c.label).toBe("usage_snapshot");
  });
});
