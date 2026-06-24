//! Best-effort verb extraction (Task 3). Lenient by construction — a bad parse
//! yields `[]`, never an error, so discovery never fails on a malformed skill.
//!
//! Priority (the scanner applies #1; this module owns #2/#3):
//!   1. A plugin-declared `commands/` dir → those command names are the verbs.
//!      (Handled by the scanner, which has the directory listing.)
//!   2. A SKILL.md router table whose header has a "Verb" column → first column.
//!   3. No recognizable pattern → `[]`.

/// Extract verbs from a SKILL.md body by looking for a markdown table whose
/// header row contains a cell equal (case-insensitively) to "verb". Returns the
/// first-column cell of each data row (skipping the `|---|` separator). Empty
/// when no such table exists or the table is malformed.
pub fn verbs_from_router_table(markdown: &str) -> Vec<String> {
    let lines: Vec<&str> = markdown.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if !is_table_row(line) {
            continue;
        }
        let header = split_row(line);
        let verb_col = header
            .iter()
            .position(|c| c.trim().eq_ignore_ascii_case("verb"));
        let Some(col) = verb_col else { continue };
        // The next line must be a separator row (| --- | --- |) for this to be a
        // real table header; otherwise keep scanning.
        let Some(sep) = lines.get(i + 1) else { continue };
        if !is_separator_row(sep) {
            continue;
        }
        // Collect the verb column from the data rows that follow.
        let mut verbs = Vec::new();
        for data in &lines[i + 2..] {
            if !is_table_row(data) {
                break; // table ended
            }
            let cells = split_row(data);
            if let Some(cell) = cells.get(col) {
                let v = clean_cell(cell);
                if !v.is_empty() {
                    verbs.push(v);
                }
            }
        }
        if !verbs.is_empty() {
            return verbs;
        }
    }
    Vec::new()
}

fn is_table_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// A separator row is a table row whose every non-empty cell is only `-`, `:`,
/// or whitespace (e.g. `| --- | :--: |`).
fn is_separator_row(line: &str) -> bool {
    if !is_table_row(line) {
        return false;
    }
    let cells = split_row(line);
    !cells.is_empty()
        && cells
            .iter()
            .all(|c| !c.trim().is_empty() && c.trim().chars().all(|ch| ch == '-' || ch == ':'))
}

/// Split a markdown table row into cells, dropping the leading/trailing empties
/// created by the bounding pipes.
fn split_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let inner = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .strip_suffix('|')
        .unwrap_or(trimmed);
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

/// Strip markdown noise (`code`, **bold**, leading `/`) from a verb cell so the
/// stored verb is the bare word.
fn clean_cell(cell: &str) -> String {
    cell.trim()
        .trim_matches('`')
        .trim_matches('*')
        .trim_start_matches('/')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_verbs_from_a_ddd_council_shaped_router_table() {
        let md = "\
# ddd-council

Routes:

| Verb | Reference | Notes |
| --- | --- | --- |
| vet | refs/vet.md | check a plan |
| critique | refs/crit.md | tear it apart |
| map | refs/map.md | context map |
";
        assert_eq!(
            verbs_from_router_table(md),
            vec!["vet".to_string(), "critique".into(), "map".into()]
        );
    }

    #[test]
    fn verbless_skill_returns_empty() {
        let md = "# brainstorming\n\nJust a description, no table.\n";
        assert!(verbs_from_router_table(md).is_empty());
    }

    #[test]
    fn a_table_without_a_verb_header_is_ignored() {
        let md = "\
| Name | Path |
| --- | --- |
| foo | a.md |
";
        assert!(verbs_from_router_table(md).is_empty());
    }

    #[test]
    fn a_malformed_table_never_panics_and_returns_empty() {
        // Header claims a Verb column but there's no separator row → not a table.
        let md = "| Verb | x\nnonsense\n| | |";
        assert!(verbs_from_router_table(md).is_empty());
    }

    #[test]
    fn cleans_code_ticks_and_leading_slash_from_cells() {
        let md = "\
| Verb | Ref |
| --- | --- |
| `vet` | a |
| /critique | b |
";
        assert_eq!(verbs_from_router_table(md), vec!["vet".to_string(), "critique".into()]);
    }

    #[test]
    fn verb_column_can_be_non_first_and_case_insensitive() {
        let md = "\
| Ref | VERB |
| --- | --- |
| a.md | vet |
| b.md | map |
";
        assert_eq!(verbs_from_router_table(md), vec!["vet".to_string(), "map".into()]);
    }
}
