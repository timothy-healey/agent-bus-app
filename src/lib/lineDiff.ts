/// One classified line of a two-version comparison.
export interface DiffLine {
  kind: "same" | "added" | "removed";
  text: string;
}

export interface LineDiff {
  added: number;
  removed: number;
  lines: DiffLine[];
}

function split(src: string): string[] {
  const norm = src.replace(/\r\n/g, "\n");
  const lines = norm.split("\n");
  // A single trailing newline yields a final "" — drop it so it isn't a "line".
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/// Classic LCS line diff. Pure: classifies every line of `left`/`right` as
/// same / removed (only in left) / added (only in right). No markdown
/// semantics, just line identity — a display-only computation for the
/// side-by-side compare badge (B2). NOT comment re-anchoring (that is B1, a
/// Review domain concept); this never decides which comments carry forward.
export function lineDiff(left: string, right: string): LineDiff {
  const a = split(left);
  const b = split(right);
  const n = a.length;
  const m = b.length;

  // LCS length table.
  const lcs: number[][] = Array.from({ length: n + 1 }, () =>
    new Array<number>(m + 1).fill(0),
  );
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j]
        ? lcs[i + 1][j + 1] + 1
        : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }

  const lines: DiffLine[] = [];
  let added = 0;
  let removed = 0;
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      lines.push({ kind: "same", text: a[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      lines.push({ kind: "removed", text: a[i] });
      removed++;
      i++;
    } else {
      lines.push({ kind: "added", text: b[j] });
      added++;
      j++;
    }
  }
  while (i < n) {
    lines.push({ kind: "removed", text: a[i] });
    removed++;
    i++;
  }
  while (j < m) {
    lines.push({ kind: "added", text: b[j] });
    added++;
    j++;
  }

  return { added, removed, lines };
}
