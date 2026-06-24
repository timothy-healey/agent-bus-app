import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ReactFlowProvider } from "@xyflow/react";
import { NodeShell, nodeTypes } from "./CanvasNodes";
import type { FlowNodeData } from "../draftFlow";
import { NODE_KINDS } from "../mutations";

function renderShell(data: FlowNodeData, kind: Parameters<typeof NodeShell>[0]["kind"]) {
  return render(
    <ReactFlowProvider>
      <NodeShell data={data} selected={false} kind={kind} />
    </ReactFlowProvider>,
  );
}

describe("canvas node renderers", () => {
  it("nodeTypes covers every NodeKind (vet F9)", () => {
    for (const k of NODE_KINDS) expect(nodeTypes[k]).toBeTypeOf("function");
  });

  it("a team node renders its label and role sublabel", () => {
    renderShell({ kind: "team", label: "Spec Writers", role: "producer", warnings: [] }, "team");
    expect(screen.getByText("Spec Writers")).toBeInTheDocument();
    expect(screen.getByText("producer")).toBeInTheDocument();
  });

  it("a reviewer team shows the reviewer sublabel", () => {
    renderShell({ kind: "team", label: "QA", role: "reviewer", warnings: [] }, "team");
    expect(screen.getByText("reviewer")).toBeInTheDocument();
  });

  it("renders a validation badge when there are warnings", () => {
    renderShell({ kind: "team", label: "A", role: "producer", warnings: ["no prompt yet"] }, "team");
    expect(screen.getByRole("img", { name: /no prompt yet/i })).toBeInTheDocument();
  });

  it("a gate node renders the human-gate sublabel", () => {
    renderShell({ kind: "gate", label: "Plan review", warnings: [] }, "gate");
    expect(screen.getByText("Plan review")).toBeInTheDocument();
    expect(screen.getByText("human gate")).toBeInTheDocument();
  });
});
