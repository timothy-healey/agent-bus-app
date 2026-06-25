import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { NodeContextMenu } from "./NodeContextMenu";

describe("NodeContextMenu (G9)", () => {
  it("renders the actions as a roled menu", () => {
    render(
      <NodeContextMenu
        x={10}
        y={20}
        nodeId="research"
        actions={[{ id: "delete", label: "Delete", onSelect: () => {}, destructive: true }]}
        onClose={() => {}}
      />,
    );
    expect(screen.getByRole("menu", { name: /actions for research/i })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: /delete/i })).toBeInTheDocument();
  });

  it("clicking an action fires onSelect then onClose", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    render(
      <NodeContextMenu
        x={0}
        y={0}
        nodeId="research"
        actions={[{ id: "delete", label: "Delete", onSelect, destructive: true }]}
        onClose={onClose}
      />,
    );
    fireEvent.click(screen.getByRole("menuitem", { name: /delete/i }));
    expect(onSelect).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("Escape closes the menu", () => {
    const onClose = vi.fn();
    render(
      <NodeContextMenu
        x={0}
        y={0}
        nodeId="research"
        actions={[{ id: "delete", label: "Delete", onSelect: () => {} }]}
        onClose={onClose}
      />,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("clicking outside closes the menu", () => {
    const onClose = vi.fn();
    render(
      <div>
        <button type="button">outside</button>
        <NodeContextMenu
          x={0}
          y={0}
          nodeId="research"
          actions={[{ id: "delete", label: "Delete", onSelect: () => {} }]}
          onClose={onClose}
        />
      </div>,
    );
    fireEvent.mouseDown(screen.getByRole("button", { name: /outside/i }));
    expect(onClose).toHaveBeenCalled();
  });
});
