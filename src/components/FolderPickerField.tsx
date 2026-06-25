import { useState, type CSSProperties } from "react";
import { Button } from "./ui/Button";
import { pickFolder } from "../ipc/workspace";

export interface FolderPickerFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
}

/// Label + an app-styled path input + a "Browse…" button that opens the NATIVE OS
/// folder dialog (Finder) via the `pickFolder` seam (A3). The OS dialog is the
/// familiar picker; only the path-display box is themed to match the app. Manual
/// ~-path text entry stays as the fallback (backend-expanded at create/set).
/// Reused for Root path and Target repo (wizard Basics + Settings).
export function FolderPickerField({ label, value, onChange, placeholder, disabled }: FolderPickerFieldProps) {
  const [busy, setBusy] = useState(false);

  async function browse() {
    setBusy(true);
    try {
      const picked = await pickFolder();
      if (picked) onChange(picked);
    } finally {
      setBusy(false);
    }
  }

  return (
    <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <span style={{ fontSize: 12, color: "var(--text-2)" }}>{label}</span>
      <div style={{ display: "flex", gap: 8 }}>
        <input
          aria-label={label}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          disabled={disabled}
          style={inp}
        />
        <Button onClick={browse} disabled={disabled || busy} aria-label={`Browse for ${label}`}>
          {busy ? "Opening…" : "Browse…"}
        </Button>
      </div>
    </label>
  );
}

const inp: CSSProperties = {
  flex: 1,
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  color: "var(--text)",
  padding: "var(--sp-2)",
  borderRadius: "var(--r-sm)",
  fontFamily: "inherit",
  fontSize: "var(--ts-base)",
};
