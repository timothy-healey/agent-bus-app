import { describe, expect, it, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ThemeToggle } from "./ThemeToggle";

describe("ThemeToggle", () => {
  beforeEach(() => {
    document.documentElement.removeAttribute("data-theme");
    localStorage.clear();
  });

  it("renders a button labelled with the current theme target", () => {
    document.documentElement.setAttribute("data-theme", "dark");
    render(<ThemeToggle />);
    expect(screen.getByRole("button")).toHaveAccessibleName(/light mode/i);
  });

  it("toggles data-theme on click", () => {
    document.documentElement.setAttribute("data-theme", "dark");
    render(<ThemeToggle />);
    fireEvent.click(screen.getByRole("button"));
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("persists the chosen theme in localStorage", () => {
    document.documentElement.setAttribute("data-theme", "dark");
    render(<ThemeToggle />);
    fireEvent.click(screen.getByRole("button"));
    expect(localStorage.getItem("agent-bus-app-theme")).toBe("light");
  });
});
