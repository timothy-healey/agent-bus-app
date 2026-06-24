import { insertForm, type SkillEntry } from "../../ipc/skills";

/// Pure helpers for SkillAutocomplete (A4). Token scan / match / insert-form /
/// highlight-token detection — all unit-tested without the DOM.

/** A `/`-trigger found at the caret: the slash position and the partial query
 *  typed after it (may be empty). Null when the caret isn't inside a `/`-token. */
export interface TriggerToken {
  /** Index of the `/` in the text. */
  slash: number;
  /** The text between the `/` and the caret (the filter query). */
  query: string;
}

/** Characters that may appear in a skill token after the leading slash. Allows
 *  the qualified `namespace:name` form and the verb-cascade `name verb` is NOT
 *  part of the token (a space closes it). */
const TOKEN_CHAR = /[A-Za-z0-9:_-]/;

/** Scan backwards from `caret` to find an active `/`-trigger. A trigger is a `/`
 *  that begins at the start of the text or after whitespace, with only token
 *  characters between it and the caret. Returns null otherwise (so a `/` inside
 *  a path like `a/b` or mid-word does not open the popover). */
export function findTrigger(text: string, caret: number): TriggerToken | null {
  let i = caret - 1;
  // Walk back over token characters.
  while (i >= 0 && TOKEN_CHAR.test(text[i])) i--;
  // The char at i must be the slash.
  if (i < 0 || text[i] !== "/") return null;
  // The slash must be at the start or preceded by whitespace.
  if (i > 0 && !/\s/.test(text[i - 1])) return null;
  return { slash: i, query: text.slice(i + 1, caret) };
}

/** Rank + filter catalog entries against a query (case-insensitive). Empty query
 *  returns all entries. Prefix matches rank above substring matches; within a
 *  tier, entries are ordered by name. Matches both the bare name and the
 *  qualified namespace:name. */
export function matchEntries(entries: SkillEntry[], query: string): SkillEntry[] {
  const q = query.toLowerCase();
  if (q === "") {
    return [...entries].sort((a, b) => a.name.localeCompare(b.name));
  }
  const scored = entries
    .map((e) => ({ e, score: scoreEntry(e, q) }))
    .filter((s) => s.score > 0);
  scored.sort((a, b) => b.score - a.score || a.e.name.localeCompare(b.e.name));
  return scored.map((s) => s.e);
}

function scoreEntry(e: SkillEntry, q: string): number {
  const name = e.name.toLowerCase();
  const qualified = insertForm({ ...e, qualified: true }).toLowerCase();
  const ns = e.namespace?.toLowerCase() ?? "";
  if (name.startsWith(q)) return 3;
  if (qualified.startsWith(q) || ns.startsWith(q)) return 2;
  if (name.includes(q) || qualified.includes(q)) return 1;
  return 0;
}

/** The text to insert for `entry` (no leading slash) — re-exported from the IPC
 *  helper so the component imports one module. */
export { insertForm };

/** Replace the active `/`-token at `[trigger.slash, caret)` with the entry's
 *  `/insert-form`, returning the new text + the caret position after insertion.
 *  When `verb` is given it appends ` <verb>` (the verb cascade). */
export function applyInsertion(
  text: string,
  caret: number,
  trigger: TriggerToken,
  entry: SkillEntry,
  verb?: string,
): { text: string; caret: number } {
  const before = text.slice(0, trigger.slash);
  const after = text.slice(caret);
  const token = `/${insertForm(entry)}${verb ? ` ${verb}` : ""}`;
  // Add a trailing space so the author can keep typing, unless one already
  // immediately follows.
  const sep = after.startsWith(" ") ? "" : " ";
  const inserted = `${token}${sep}`;
  return { text: `${before}${inserted}${after}`, caret: before.length + inserted.length };
}

/** A `/skill` token found in the body, for mirror-overlay highlighting. */
export interface HighlightToken {
  start: number;
  end: number;
  text: string;
  /** The token's name part after the slash (sans any verb), for catalog lookup. */
  name: string;
}

/** Scan the whole body for `/token` occurrences (slash at start or after
 *  whitespace). Used by the mirror overlay to decide which spans to tint. */
export function scanTokens(text: string): HighlightToken[] {
  const tokens: HighlightToken[] = [];
  const re = /(^|\s)(\/[A-Za-z0-9:_-]+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    const lead = m[1].length;
    const start = m.index + lead;
    const tok = m[2];
    tokens.push({ start, end: start + tok.length, text: tok, name: tok.slice(1) });
  }
  return tokens;
}

/** True when `name` (the part after the slash, possibly qualified) matches some
 *  catalog entry — by bare name or by its qualified namespace:name form. Drives
 *  the "recognized → tinted" mirror-overlay rule. */
export function isRecognized(name: string, entries: SkillEntry[]): boolean {
  return entries.some(
    (e) =>
      e.name === name ||
      (e.namespace != null && `${e.namespace}:${e.name}` === name),
  );
}
