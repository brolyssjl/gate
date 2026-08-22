//! Port of `src/artifacts/frontmatter.ts` (`splitFrontmatter`, the shared
//! YAML-frontmatter split used by every artifact parser). Ported in wave 3
//! (see `docs/rust-port.md`), built on `core::yaml` (wave 1, already
//! covers the block-sequence-of-maps shapes this needs - see its module
//! doc comment).
//!
//! TS: a leading `---`-fenced YAML block followed by a free-form markdown
//! body. Each artifact's own parser still owns its field-level validation -
//! this only owns the mechanical split and its three failure modes, which
//! were previously copy-pasted four times.

use crate::core::yaml::{self, YamlValue};

#[derive(Debug, Clone, PartialEq)]
pub struct FrontmatterSplit {
    pub data: YamlValue,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FrontmatterResult {
    Ok(FrontmatterSplit),
    Err(Vec<String>),
}

/// Anchored match of the frontmatter regex `^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$`:
/// a `---` line, the shortest run of lines up to the next `---` line (the
/// YAML block), then everything after. Returns `(yaml_block, body)`.
fn split_frontmatter_raw(raw: &str) -> Option<(String, String)> {
    let after_open = raw
        .strip_prefix("---\r\n")
        .or_else(|| raw.strip_prefix("---\n"))?;
    // Find the first line that is exactly "---" (optionally "\r"), scanning
    // line by line so the yaml block is the *shortest* match (mirrors the
    // regex's non-greedy `[\s\S]*?`).
    let mut search_from = 0usize;
    loop {
        let rest = &after_open[search_from..];
        let nl = rest.find('\n');
        let (line, line_end) = match nl {
            Some(idx) => (&rest[..idx], search_from + idx + 1),
            None => (rest, after_open.len()),
        };
        let line_trimmed = line.strip_suffix('\r').unwrap_or(line);
        if line_trimmed == "---" {
            // yaml block is everything before this line (its own trailing
            // \r?\n was already excluded by `search_from`); body is
            // everything after the closing fence's own line ending.
            let yaml_block = &after_open[..search_from];
            let body = &after_open[line_end..];
            return Some((yaml_block.to_string(), body.to_string()));
        }
        nl?;
        search_from = line_end;
    }
}

/// `fileLabel` (e.g. "plan.md") names the file in the "must start with..." error.
pub fn split_frontmatter(raw: &str, file_label: &str) -> FrontmatterResult {
    let Some((yaml_block, body)) = split_frontmatter_raw(raw) else {
        return FrontmatterResult::Err(vec![format!(
            "{file_label} must start with a YAML frontmatter block (---)"
        )]);
    };
    let fm = match yaml::parse_yaml(&yaml_block) {
        Ok(v) => v,
        Err(e) => {
            return FrontmatterResult::Err(vec![format!("frontmatter is not valid YAML: {}", e.0)]);
        }
    };
    if fm.is_null() || fm.as_map().is_none() {
        return FrontmatterResult::Err(vec!["frontmatter must be a YAML mapping".to_string()]);
    }
    FrontmatterResult::Ok(FrontmatterSplit {
        data: fm,
        body: body.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_well_formed_frontmatter_block_from_the_body() {
        let raw = "---\ngoal: g\n---\n# Body\ntext\n";
        match split_frontmatter(raw, "plan.md") {
            FrontmatterResult::Ok(split) => {
                assert_eq!(split.data.get("goal").unwrap().as_str(), Some("g"));
                assert_eq!(split.body, "# Body\ntext");
            }
            FrontmatterResult::Err(e) => panic!("expected ok, got {e:?}"),
        }
    }

    #[test]
    fn rejects_input_with_no_leading_frontmatter_fence() {
        match split_frontmatter("# just markdown", "plan.md") {
            FrontmatterResult::Err(errors) => {
                assert!(errors[0].contains("plan.md must start with a YAML frontmatter block"));
            }
            FrontmatterResult::Ok(_) => panic!("expected an error"),
        }
    }

    #[test]
    fn rejects_invalid_yaml_in_the_frontmatter_block() {
        match split_frontmatter("---\nsource: |\n  a\n---\nbody", "plan.md") {
            FrontmatterResult::Err(errors) => {
                assert!(errors[0].contains("frontmatter is not valid YAML"));
            }
            FrontmatterResult::Ok(_) => panic!("expected an error"),
        }
    }

    #[test]
    fn rejects_frontmatter_that_is_not_a_mapping() {
        match split_frontmatter("---\n- a\n- b\n---\nbody", "plan.md") {
            FrontmatterResult::Err(errors) => {
                assert_eq!(errors[0], "frontmatter must be a YAML mapping");
            }
            FrontmatterResult::Ok(_) => panic!("expected an error"),
        }
    }

    #[test]
    fn treats_an_empty_frontmatter_block_as_not_a_mapping() {
        // yaml package parses an empty document to `null`, not `{}`.
        match split_frontmatter("---\n---\nbody", "plan.md") {
            FrontmatterResult::Err(errors) => {
                assert_eq!(errors[0], "frontmatter must be a YAML mapping");
            }
            FrontmatterResult::Ok(_) => panic!("expected an error"),
        }
    }

    #[test]
    fn trims_the_body_and_defaults_to_empty_when_absent() {
        match split_frontmatter("---\ngoal: g\n---\n", "plan.md") {
            FrontmatterResult::Ok(split) => assert_eq!(split.body, ""),
            FrontmatterResult::Err(e) => panic!("expected ok, got {e:?}"),
        }
    }

    #[test]
    fn handles_crlf_line_endings() {
        let raw = "---\r\ngoal: g\r\n---\r\n# Body\r\n";
        match split_frontmatter(raw, "plan.md") {
            FrontmatterResult::Ok(split) => {
                assert_eq!(split.data.get("goal").unwrap().as_str(), Some("g"));
            }
            FrontmatterResult::Err(e) => panic!("expected ok, got {e:?}"),
        }
    }
}
