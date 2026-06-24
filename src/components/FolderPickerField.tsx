import { useState } from "react";
import { Button } from "./ui/Button";
import { pickFolder } from "../ipc/workspace";

export interface FolderPickerFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
}

/** Label + text input + a "Browse…" button that opens the native folder picker
 *  (A3). The picker returns an absolute path written into the field; typing a
 *  ~-path by hand still works (expanded backend-side at create / set). Reused for
 *  Root path and Target repo (wizard Basics + Settings). Depends on the
 *  `pickFolder()` IPC wrapper, never the dialog plugin directly. */
export function FolderPickerField({
  label,
  value,
  onChange,
  placeholder,
  disabled,
}: FolderPickerFieldProps) {
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
          disabled={disabled || busy}
          style={{ flex: 1 }}
        />
        <Button
          onClick={browse}
          disabled={disabled || busy}
          aria-label={`Browse for ${label}`}
        >
          {busy ? "Browsing…" : "Browse…"}
        </Button>
      </div>
    </label>
  );
}
