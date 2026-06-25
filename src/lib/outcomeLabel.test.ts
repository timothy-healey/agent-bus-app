import { describe, expect, it } from "vitest";
import { outcomeLabel, isErrorOutcome } from "./outcomeLabel";

describe("outcomeLabel", () => {
  it("humanizes verdicts", () => {
    expect(outcomeLabel("verdict:approve")).toBe("approved");
    expect(outcomeLabel("verdict:revise")).toBe("revise requested");
    expect(outcomeLabel("verdict:reject")).toBe("rejected");
  });

  it("humanizes error classes", () => {
    expect(outcomeLabel("error:rate_limited")).toBe("rate limited");
    expect(outcomeLabel("error:model_unavailable")).toBe("model unavailable");
    expect(outcomeLabel("error:spawn")).toBe("could not start");
    expect(outcomeLabel("error:no_result")).toBe("no result");
    expect(outcomeLabel("error:other")).toBe("failed");
  });

  it("labels an in-flight (empty) outcome", () => {
    expect(outcomeLabel("")).toBe("in flight");
  });

  it("has no em-dashes or exclamation marks (copy pass)", () => {
    for (const o of ["verdict:approve", "verdict:reject", "error:rate_limited", "error:other", ""]) {
      expect(outcomeLabel(o)).not.toMatch(/[—!]/);
    }
  });

  it("isErrorOutcome distinguishes errors from verdicts", () => {
    expect(isErrorOutcome("error:spawn")).toBe(true);
    expect(isErrorOutcome("verdict:approve")).toBe(false);
    expect(isErrorOutcome("")).toBe(false);
  });
});
