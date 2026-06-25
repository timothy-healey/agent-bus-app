/// Roving-tabindex key math for a horizontal WAI-ARIA tablist. Given the pressed
/// key, the current active index, and the tab count, returns the next index to
/// activate+focus, or null when the key is not a navigation key (caller should
/// NOT preventDefault). Wraps at both ends. No-op (null) on an empty tablist.
export function rovingTabKey(
  key: string,
  index: number,
  count: number,
): number | null {
  if (count <= 0) return null;
  switch (key) {
    case "ArrowRight":
      return (index + 1) % count;
    case "ArrowLeft":
      return (index - 1 + count) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}
