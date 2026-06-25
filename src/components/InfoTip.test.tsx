import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { InfoTip } from "./InfoTip";

describe("InfoTip", () => {
  it("toggles a tooltip popover on click and wires aria-describedby", () => {
    render(<InfoTip text="helpful detail" label="help for Reads" />);
    const btn = screen.getByRole("button", { name: "help for Reads" });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    expect(btn).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(btn);
    const tip = screen.getByRole("tooltip");
    expect(tip).toHaveTextContent("helpful detail");
    expect(btn).toHaveAttribute("aria-expanded", "true");
    expect(btn).toHaveAttribute("aria-describedby", tip.id);
  });

  it("dismisses on Escape", () => {
    render(<InfoTip text="x" />);
    fireEvent.click(screen.getByRole("button"));
    expect(screen.getByRole("tooltip")).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("tooltip"), { key: "Escape" });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });
});
