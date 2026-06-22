import type { CSSProperties, ReactNode } from "react";

export type ButtonVariant = "default" | "primary" | "ghost" | "danger";

export interface ButtonProps {
  children: ReactNode;
  variant?: ButtonVariant;
  size?: "default" | "sm";
  disabled?: boolean;
  onClick?: () => void;
  title?: string;
}

const base: CSSProperties = {
  fontFamily: "inherit",
  borderRadius: "var(--r-sm)",
  border: "1px solid",
  cursor: "pointer",
  fontSize: 11.5,
};

const variants: Record<ButtonVariant, CSSProperties> = {
  default: {
    background: "var(--surface-3)",
    borderColor: "var(--border-2)",
    color: "var(--text)",
  },
  primary: {
    background: "var(--accent)",
    borderColor: "var(--accent)",
    color: "oklch(15% 0.04 55)",
    fontWeight: 500,
  },
  ghost: {
    background: "transparent",
    borderColor: "var(--border)",
    color: "var(--text-2)",
  },
  danger: {
    background: "transparent",
    borderColor: "oklch(35% 0.05 25)",
    color: "var(--danger)",
  },
};

export function Button({
  children,
  variant = "default",
  size = "default",
  disabled = false,
  onClick,
  title,
}: ButtonProps) {
  const sizing: CSSProperties =
    size === "sm" ? { padding: "4px 10px" } : { padding: "7px 14px" };
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={disabled ? undefined : onClick}
      style={{
        ...base,
        ...sizing,
        ...variants[variant],
        opacity: disabled ? 0.5 : 1,
        cursor: disabled ? "not-allowed" : "pointer",
      }}
    >
      {children}
    </button>
  );
}
