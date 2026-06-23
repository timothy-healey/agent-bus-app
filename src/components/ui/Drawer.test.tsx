import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Drawer } from "./Drawer";

describe("Drawer", () => {
  it("renders nothing when closed", () => {
    const { container } = render(
      <Drawer open={false} onClose={() => {}}>
        body
      </Drawer>,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders children when open", () => {
    render(
      <Drawer open onClose={() => {}}>
        <div>drawer body</div>
      </Drawer>,
    );
    expect(screen.getByText("drawer body")).toBeInTheDocument();
  });

  it("calls onClose when the backdrop is clicked", () => {
    const onClose = vi.fn();
    render(
      <Drawer open onClose={onClose}>
        body
      </Drawer>,
    );
    fireEvent.click(screen.getByTestId("drawer-backdrop"));
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("calls onClose when the close button is clicked", () => {
    const onClose = vi.fn();
    render(
      <Drawer open onClose={onClose}>
        body
      </Drawer>,
    );
    fireEvent.click(screen.getByRole("button", { name: /close/i }));
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("exposes a dialog role with aria-modal and an accessible name", () => {
    render(
      <Drawer open onClose={() => {}} label="task T-1">
        body
      </Drawer>,
    );
    const dialog = screen.getByRole("dialog", { name: /task T-1/i });
    expect(dialog).toHaveAttribute("aria-modal", "true");
  });

  it("closes on Escape (focus trap a11y)", () => {
    const onClose = vi.fn();
    render(
      <Drawer open onClose={onClose}>
        body
      </Drawer>,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("moves focus into the drawer on open", () => {
    render(
      <Drawer open onClose={() => {}}>
        <button>inside</button>
      </Drawer>,
    );
    // first focusable is the close button
    expect(document.activeElement?.getAttribute("aria-label")).toBe("close");
  });
});
