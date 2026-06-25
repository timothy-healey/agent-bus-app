import { useState, type CSSProperties } from "react";
import { Button } from "./ui/Button";
import { FileTreePicker } from "./FileTreePicker";

export interface FolderPickerFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  /// Absolute base the in-app tree is rooted at. Defaults to "~" (home).
  root?: string;
}

/// Label + text input + a "Browse…" toggle that opens the in-app themed
/// `FileTreePicker` (G7, single-folder mode) — replacing the native OS dialog,
/// which couldn't be themed. Manual ~-path text entry stays as the fallback
/// (still backend-expanded at create/set). Reused for Root path and Target repo
/// (wizard Basics + Settings).
export function FolderPickerField({
  label,
  value,
  onChange,
  placeholder,
  disabled,
  root = "~",
}: FolderPickerFieldProps) {
  const [open, setOpen] = useState(false);
  // Single-select folder mode: selection is [value] when value is absolute.
  const selected = value && value.startsWith("/") ? [value] : [];

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
          style={{ flex: 1 }}
        />
        <Button
          onClick={() => setOpen((o) => !o)}
          disabled={disabled}
          aria-label={`Browse for ${label}`}
          aria-expanded={open}
        >
          {open ? "Close" : "Browse…"}
        </Button>
      </div>
      {open && (
        <div style={picker}>
          <FileTreePicker
            root={root}
            mode="folder"
            label={`folder picker for ${label}`}
            selected={selected}
            onChange={(sel) => {
              const next = sel[0] ?? "";
              if (next) onChange(next);
            }}
          />
          <div style={{ fontSize: "var(--ts-xs)", color: "var(--text-3)", marginTop: 4 }}>
            Pick a folder, or type a path above. Selected: {value || "(none)"}
          </div>
        </div>
      )}
    </label>
  );
}

const picker: CSSProperties = { marginTop: 4 };
