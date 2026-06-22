import { useTheme } from "../hooks/useTheme";

export function ThemeToggle() {
  const [theme, setTheme] = useTheme();
  const next: "dark" | "light" = theme === "dark" ? "light" : "dark";
  const label = `${next === "light" ? "Light" : "Dark"} mode`;

  return (
    <button
      onClick={() => setTheme(next)}
      aria-label={label}
      style={{
        background: "transparent",
        border: "1px solid var(--border)",
        color: "var(--text-2)",
        padding: "4px 10px",
        borderRadius: "var(--r-sm)",
        fontFamily: "inherit",
        fontSize: "11px",
        cursor: "pointer",
      }}
    >
      {label}
    </button>
  );
}
