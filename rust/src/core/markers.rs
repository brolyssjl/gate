//! Port of `src/core/markers.ts`: managed-block injection for agent config
//! files (adapters write into `CLAUDE.md`/`AGENTS.md`/etc. between these
//! markers, preserving everything else the user wrote byte-for-byte).
//! Rewriting is idempotent: running it twice yields an identical file.

pub const START_MARKER: &str = "<!-- gate:start -->";
pub const END_MARKER: &str = "<!-- gate:end -->";

/// Wrap block body in markers with a note that the region is tool-managed.
pub fn wrap_managed_block(body: &str) -> String {
    format!(
        "{START_MARKER}\n<!-- Managed by gate. Edits inside this block are overwritten on `gate adapt`. -->\n{}\n{END_MARKER}",
        body.trim_end()
    )
}

/// Whether the first `START_MARKER` in `text` closes with an `END_MARKER`
/// before any other `START_MARKER` begins.
fn has_complete_first_block(text: &str) -> bool {
    let Some(start) = text.find(START_MARKER) else {
        return false;
    };
    let Some(end_rel) = text[start..].find(END_MARKER) else {
        return false; // orphaned start marker: no end anywhere after it
    };
    let end = start + end_rel;
    match text[start + START_MARKER.len()..].find(START_MARKER) {
        Some(next_rel) => start + START_MARKER.len() + next_rel > end,
        None => true,
    }
}

/// Find every non-overlapping `START_MARKER..END_MARKER` span (lazy,
/// left-to-right), mirroring the TS `BLOCK_RE` global match.
fn find_blocks(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut search_from = 0usize;
    while let Some(start_rel) = text[search_from..].find(START_MARKER) {
        let start = search_from + start_rel;
        match text[start..].find(END_MARKER) {
            Some(end_rel) => {
                let end = start + end_rel + END_MARKER.len();
                spans.push((start, end));
                search_from = end;
            }
            None => break, // orphaned start marker with no end anywhere after it
        }
    }
    spans
}

/// Trim trailing whitespace, mirroring `existing.replace(/\s*$/, "")`.
fn trim_trailing_ws(s: &str) -> &str {
    s.trim_end()
}

/// Collapse 3+ consecutive newlines down to exactly 2, mirroring
/// `result.replace(/\n{3,}/g, "\n\n")`.
fn collapse_blank_runs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = 0usize;
    for ch in s.chars() {
        if ch == '\n' {
            run += 1;
            if run <= 2 {
                out.push(ch);
            }
        } else {
            run = 0;
            out.push(ch);
        }
    }
    out
}

/// Insert or replace the managed block in `existing`. When no complete block
/// is present the managed block is appended (with one blank-line separator)
/// so user content stays on top. When present, the first block is replaced
/// in place and any stray duplicate blocks are removed.
pub fn upsert_managed_block(existing: &str, body: &str) -> String {
    let managed = wrap_managed_block(body);
    if !has_complete_first_block(existing) {
        let base = trim_trailing_ws(existing);
        return if base.is_empty() {
            format!("{managed}\n")
        } else {
            format!("{base}\n\n{managed}\n")
        };
    }

    let spans = find_blocks(existing);
    let mut result = String::with_capacity(existing.len());
    let mut cursor = 0usize;
    for (i, (start, end)) in spans.iter().enumerate() {
        result.push_str(&existing[cursor..*start]);
        if i == 0 {
            result.push_str(&managed);
        }
        // duplicates (i > 0) contribute nothing, collapsing accidental repeats
        cursor = *end;
    }
    result.push_str(&existing[cursor..]);

    format!("{}\n", trim_trailing_ws(&collapse_blank_runs(&result)))
}

/// Whether `existing` contains at least one complete (matched) managed
/// block - an orphaned start marker with no end anywhere after it does not
/// count, mirroring the TS lazy-regex `.test()`.
pub fn has_managed_block(existing: &str) -> bool {
    !find_blocks(existing).is_empty()
}

/// Whether the first managed block in `existing` already carries `body` -
/// i.e. a `gate adapt`/`update` run with this same body would report
/// "unchanged". `false` when there is no complete block at all (use
/// `has_managed_block` to tell that apart from "present but stale", the
/// distinction `core::doctor` reports separately).
pub fn managed_block_matches(existing: &str, body: &str) -> bool {
    match find_blocks(existing).first() {
        Some(&(start, end)) => existing[start..end] == wrap_managed_block(body),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_a_managed_block_to_user_content_preserving_it() {
        let existing = "# My rules\n\nAlways be nice.\n";
        let out = upsert_managed_block(existing, "BODY");
        assert!(out.starts_with("# My rules\n\nAlways be nice."));
        assert!(out.contains(START_MARKER));
        assert!(out.contains(END_MARKER));
        assert!(out.contains("BODY"));
    }

    #[test]
    fn is_idempotent_running_twice_yields_identical_output() {
        let existing = "# My rules\n\nkeep me\n";
        let once = upsert_managed_block(existing, "BODY v1");
        let twice = upsert_managed_block(&once, "BODY v1");
        assert_eq!(once, twice);
    }

    #[test]
    fn replaces_the_block_body_without_touching_surrounding_user_content() {
        let existing = "top\n";
        let v1 = upsert_managed_block(existing, "OLD");
        let v2 = upsert_managed_block(&format!("{v1}\nuser added this later\n"), "NEW");
        assert!(v2.contains("NEW"));
        assert!(!v2.contains("OLD"));
        assert!(v2.starts_with("top"));
        assert!(v2.contains("user added this later"));
    }

    #[test]
    fn collapses_accidental_duplicate_blocks_into_one() {
        let block = format!("{START_MARKER}\nx\n{END_MARKER}");
        let existing = format!("a\n\n{block}\n\nb\n\n{block}\n");
        let out = upsert_managed_block(&existing, "ONE");
        let count = out.matches(START_MARKER).count();
        assert_eq!(count, 1);
        assert!(out.contains('a'));
        assert!(out.contains('b'));
    }

    #[test]
    fn managed_block_matches_is_true_only_when_the_first_block_carries_that_exact_body() {
        let existing = upsert_managed_block("top\n", "BODY v1");
        assert!(managed_block_matches(&existing, "BODY v1"));
        assert!(!managed_block_matches(&existing, "BODY v2"));
        assert!(!managed_block_matches("no block here", "BODY v1"));
    }

    #[test]
    fn handles_empty_input() {
        let out = upsert_managed_block("", "BODY");
        assert!(has_managed_block(&out));
    }

    #[test]
    fn never_deletes_user_content_around_an_orphaned_start_marker() {
        let existing = format!(
            "keep this\n\n{START_MARKER}\nnot a real block, no end marker\nstill user content\n"
        );

        let once = upsert_managed_block(&existing, "BODY v1");
        assert!(once.contains("keep this"));
        assert!(once.contains("not a real block, no end marker"));
        assert!(once.contains("still user content"));
        assert!(once.starts_with(trim_trailing_ws(&existing)));

        let twice = upsert_managed_block(&once, "BODY v2");
        assert!(twice.contains("keep this"));
        assert!(twice.contains("not a real block, no end marker"));
        assert!(twice.contains("still user content"));
        assert!(twice.starts_with(trim_trailing_ws(&existing)));
    }
}
