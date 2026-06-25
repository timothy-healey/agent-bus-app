import { describe, expect, it } from "vitest";
import { classifyNeedsHuman } from "./classifyNeedsHuman";
import type { InvocationRow } from "../ipc/runtime";
import type { Pipeline } from "../ipc/pipeline";

function row(outcome: string, over: Partial<InvocationRow> = {}): InvocationRow {
  return {
    invocation_id: "I-1", team_id: "spec", model: "m", attempts: 1,
    started_at: 10, settled_at: 20, outcome, input_tokens: 0, output_tokens: 0,
    ...over,
  };
}

function pipeline(over: Partial<Pipeline> = {}): Pipeline {
  return {
    id: "p", name: "P", description: "", schema_version: 3,
    teams: [], gates: [], escalations: [], forks: [], joins: [], ...over,
  };
}

const withEscalation = pipeline({ escalations: [{ id: "needs-human", triggers: [] }] });

describe("classifyNeedsHuman", () => {
  it("classifies the latest error outcome as a failure", () => {
    expect(classifyNeedsHuman([row("error:rate_limited")], withEscalation)).toBe("failure");
    expect(classifyNeedsHuman([row("error:model_unavailable")], withEscalation)).toBe("failure");
  });

  it("classifies a latest reject as a failure", () => {
    expect(classifyNeedsHuman([row("verdict:reject")], withEscalation)).toBe("failure");
  });

  it("classifies an exhausted revise (latest verdict:revise) as a failure", () => {
    expect(classifyNeedsHuman([row("verdict:revise")], withEscalation)).toBe("failure");
  });

  it("classifies a clean approve into a terminal escalation as a hand-off", () => {
    expect(classifyNeedsHuman([row("verdict:approve")], withEscalation)).toBe("handoff");
  });

  it("treats a clean approve as a failure when no escalation node exists", () => {
    expect(classifyNeedsHuman([row("verdict:approve")], pipeline())).toBe("failure");
  });

  it("uses the NEWEST row (index 0) — a later approve over an earlier error is a hand-off", () => {
    const rows = [row("verdict:approve", { started_at: 30 }), row("error:spawn", { started_at: 10 })];
    expect(classifyNeedsHuman(rows, withEscalation)).toBe("handoff");
  });

  it("defaults to failure on an empty trail", () => {
    expect(classifyNeedsHuman([], withEscalation)).toBe("failure");
  });

  it("defaults to failure on an in-flight (empty outcome) latest row", () => {
    expect(classifyNeedsHuman([row("")], withEscalation)).toBe("failure");
  });

  it("tolerates a null pipeline (failure)", () => {
    expect(classifyNeedsHuman([row("verdict:approve")], null)).toBe("failure");
  });
});
