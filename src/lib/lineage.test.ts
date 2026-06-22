import { describe, expect, it } from "vitest";
import { buildLineage, type LineageEntry } from "./lineage";
import type { Task } from "../ipc/runtime";

function t(over: Partial<Task>): Task {
  return {
    id: "T-1", project_id: "p", pipeline: "pipe", topic: "Bulk write",
    target_repo: null, target_scope: null, current_stage: "plan-writers",
    state: "gated", attempts: 1, parent_artifact: null, review_artifact: null,
    created_at: 0, updated_at: 0, ...over,
  };
}

describe("buildLineage", () => {
  it("always starts with the topic as the root", () => {
    const chain = buildLineage(t({}));
    expect(chain[0]).toEqual<LineageEntry>({ label: "topic", path: null, kind: "topic" });
  });

  it("includes parent_artifact when present", () => {
    const chain = buildLineage(t({ parent_artifact: "artifacts/specs/T-1-v1.md" }));
    expect(chain.some((e) => e.path === "artifacts/specs/T-1-v1.md")).toBe(true);
  });

  it("includes both parent and review artifacts, parent before review", () => {
    const chain = buildLineage(
      t({ parent_artifact: "artifacts/plans/T-1-v1.md", review_artifact: "artifacts/reviews/T-1-plan-v1.md" }),
    );
    const paths = chain.map((e) => e.path);
    expect(paths).toContain("artifacts/plans/T-1-v1.md");
    expect(paths).toContain("artifacts/reviews/T-1-plan-v1.md");
    expect(paths.indexOf("artifacts/plans/T-1-v1.md"))
      .toBeLessThan(paths.indexOf("artifacts/reviews/T-1-plan-v1.md"));
  });

  it("dedupes when parent and review are the same path", () => {
    const chain = buildLineage(
      t({ parent_artifact: "artifacts/specs/T-1-v1.md", review_artifact: "artifacts/specs/T-1-v1.md" }),
    );
    expect(chain.filter((e) => e.path === "artifacts/specs/T-1-v1.md").length).toBe(1);
  });
});
