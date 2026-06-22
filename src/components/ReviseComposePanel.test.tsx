import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ReviseComposePanel } from "./ReviseComposePanel";

describe("ReviseComposePanel", () => {
  it("summarises the comment count and resulting attempts", () => {
    render(
      <ReviseComposePanel
        commentCount={2}
        nextAttempts={2}
        target="plan-writers"
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.getByText(/2 inline comments/i)).toBeInTheDocument();
    expect(screen.getByText(/2\/3/)).toBeInTheDocument();
    expect(screen.getAllByText(/plan-writers/).length).toBeGreaterThan(0);
  });

  it("sends the typed overall direction", () => {
    const onSend = vi.fn();
    render(
      <ReviseComposePanel
        commentCount={0}
        nextAttempts={2}
        target="spec-writers"
        onSend={onSend}
        onCancel={() => {}}
      />,
    );
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "rewrite tasks 1+2" },
    });
    fireEvent.click(screen.getByRole("button", { name: /send back/i }));
    expect(onSend).toHaveBeenCalledWith("rewrite tasks 1+2");
  });

  it("sends an empty string when no direction is typed", () => {
    const onSend = vi.fn();
    render(
      <ReviseComposePanel
        commentCount={1}
        nextAttempts={3}
        target="spec-writers"
        onSend={onSend}
        onCancel={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /send back/i }));
    expect(onSend).toHaveBeenCalledWith("");
  });

  it("calls onCancel", () => {
    const onCancel = vi.fn();
    render(
      <ReviseComposePanel
        commentCount={0}
        nextAttempts={2}
        target="x"
        onSend={() => {}}
        onCancel={onCancel}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(onCancel).toHaveBeenCalledOnce();
  });
});
