import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Button } from "./Button";

describe("Button", () => {
  it("renders its label and fires onClick", () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>approve</Button>);
    fireEvent.click(screen.getByRole("button", { name: /approve/i }));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("does not fire onClick when disabled", () => {
    const onClick = vi.fn();
    render(
      <Button onClick={onClick} disabled>
        approve
      </Button>,
    );
    fireEvent.click(screen.getByRole("button", { name: /approve/i }));
    expect(onClick).not.toHaveBeenCalled();
  });

  it("uses the accent background for the primary variant", () => {
    render(<Button variant="primary">go</Button>);
    const btn = screen.getByRole("button", { name: /go/i });
    expect(btn.style.background).toContain("--accent");
  });

  it("uses the danger colour for the danger variant", () => {
    render(<Button variant="danger">reject</Button>);
    const btn = screen.getByRole("button", { name: /reject/i });
    expect(btn.style.color).toContain("--danger");
  });
});
