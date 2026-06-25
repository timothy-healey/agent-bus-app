/// Pure selection logic for the FileTreePicker (G7). Kept out of the component
/// so the toggle/normalisation rules are unit-testable without React or a
/// mocked `list_dir`. The picker holds an ordered list of selected paths;
/// these helpers add/remove/normalise them.

/// Toggle `path` in the selection. Single-select mode replaces the whole
/// selection with `[path]` (or clears it when re-picking the same one is NOT
/// desired — single-select keeps the pick). Multi-select adds or removes it,
/// preserving insertion order of the survivors.
export function toggleSelection(
  selected: string[],
  path: string,
  multi: boolean,
): string[] {
  if (!multi) return [path];
  return selected.includes(path)
    ? selected.filter((p) => p !== path)
    : [...selected, path];
}

/// Make an absolute child path relative to `base` (the project's target_repo),
/// for scope reads/writes which are stored repo-relative. Returns the absolute
/// path unchanged when it is not under `base` (caller decides whether to allow
/// that). A trailing slash on `base` is tolerated; the result never has a
/// leading slash.
export function toRepoRelative(absPath: string, base: string): string {
  if (!base) return absPath;
  const b = base.replace(/\/+$/, "");
  if (absPath === b) return ".";
  const prefix = b + "/";
  return absPath.startsWith(prefix) ? absPath.slice(prefix.length) : absPath;
}

/// Parent directory of an absolute path (used to expand the tree to a path).
/// Returns "" for a root-level entry.
export function parentOf(absPath: string): string {
  const trimmed = absPath.replace(/\/+$/, "");
  const idx = trimmed.lastIndexOf("/");
  return idx <= 0 ? "" : trimmed.slice(0, idx);
}
