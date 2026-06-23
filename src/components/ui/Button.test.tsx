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

  it("uses the primary variant class (carries accent styling in CSS)", () => {
    render(<Button variant="primary">go</Button>);
    const btn = screen.getByRole("button", { name: /go/i });
    expect(btn.className).toContain("abp-btn-primary");
    expect(btn.className).toContain("abp-btn");
  });

  it("uses the danger variant class for the danger variant", () => {
    render(<Button variant="danger">reject</Button>);
    const btn = screen.getByRole("button", { name: /reject/i });
    expect(btn.className).toContain("abp-btn-danger");
  });

  it("applies the small-size modifier class", () => {
    render(<Button size="sm">x</Button>);
    expect(screen.getByRole("button", { name: /x/i }).className).toContain("abp-btn-sm");
  });

  it("forwards aria-label", () => {
    render(<Button aria-label="save thing">x</Button>);
    expect(screen.getByRole("button", { name: /save thing/i })).toBeInTheDocument();
  });
});
