import type { ReactNode } from "react";

export type ButtonVariant = "default" | "primary" | "ghost" | "danger";

export interface ButtonProps {
  children: ReactNode;
  variant?: ButtonVariant;
  size?: "default" | "sm";
  disabled?: boolean;
  onClick?: () => void;
  title?: string;
  "aria-label"?: string;
  "aria-expanded"?: boolean;
}

/// Button primitive. Styling lives in CSS classes (global.css) so hover/focus/
/// active pseudo-states work — inline style objects cannot express them
/// (DESIGN.md §States, audit Decision 1 / C2).
export function Button({
  children,
  variant = "default",
  size = "default",
  disabled = false,
  onClick,
  title,
  "aria-label": ariaLabel,
  "aria-expanded": ariaExpanded,
}: ButtonProps) {
  const className = [
    "abp-btn",
    `abp-btn-${variant}`,
    size === "sm" ? "abp-btn-sm" : "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <button
      type="button"
      className={className}
      title={title}
      aria-label={ariaLabel}
      aria-expanded={ariaExpanded}
      disabled={disabled}
      onClick={disabled ? undefined : onClick}
    >
      {children}
    </button>
  );
}
