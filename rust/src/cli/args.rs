//! Port of `src/cli/args.ts`: minimal argv parser. Kept dependency-free so
//! `gate status`/`gate next` stay fast on the agent hot path. Unlike
//! agnosgram's `core/args.rs` (which reimplements Node's strict
//! `parseArgs`), gate's own TS parser never validates or errors - it just
//! recognizes `--flag value`, `--flag=value`, boolean `--flag`/`-f`, and
//! positionals. Ported 1:1, quirks included (e.g. a flag's value is only
//! withheld when the *next* token starts with `--`, not a single `-`).

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum FlagValue {
    Str(String),
    /// TS only ever stores the literal `true`; a flag is either absent or
    /// `true` (a value string, even the text `"false"`, is stored as
    /// `Str`).
    Bool,
}

#[derive(Debug, Clone, Default)]
pub struct Flags(HashMap<String, FlagValue>);

impl Flags {
    /// `flags.name === true` in TS: true only for the boolean form, never a
    /// string value (even `"true"`).
    pub fn is_true(&self, name: &str) -> bool {
        matches!(self.0.get(name), Some(FlagValue::Bool))
    }

    /// `typeof flags.name === "string" ? flags.name : undefined`.
    pub fn str(&self, name: &str) -> Option<&str> {
        match self.0.get(name) {
            Some(FlagValue::Str(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Whether the flag was given at all, in either form.
    pub fn is_present(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    fn insert(&mut self, key: String, value: FlagValue) {
        self.0.insert(key, value);
    }
}

#[derive(Debug, Clone, Default)]
pub struct ParsedArgs {
    pub command: Option<String>,
    pub positionals: Vec<String>,
    pub flags: Flags,
}

/// Port of `parseArgs`. The command is the first token only when it isn't a
/// flag; `gate --version` and `gate -h` have no command, just global flags.
pub fn parse_args(argv: &[String]) -> ParsedArgs {
    let mut rest: Vec<String> = argv.to_vec();
    let command = if !rest.is_empty() && !rest[0].starts_with('-') {
        Some(rest.remove(0))
    } else {
        None
    };

    let mut positionals = Vec::new();
    let mut flags = Flags::default();

    let mut i = 0usize;
    while i < rest.len() {
        let arg = &rest[i];
        if let Some(body) = arg.strip_prefix("--") {
            if let Some(eq) = body.find('=') {
                flags.insert(
                    body[..eq].to_string(),
                    FlagValue::Str(body[eq + 1..].to_string()),
                );
                i += 1;
            } else {
                let next = rest.get(i + 1);
                match next {
                    Some(n) if !n.starts_with("--") => {
                        flags.insert(body.to_string(), FlagValue::Str(n.clone()));
                        i += 2;
                    }
                    _ => {
                        flags.insert(body.to_string(), FlagValue::Bool);
                        i += 1;
                    }
                }
            }
        } else if arg.starts_with('-') && arg.len() > 1 {
            // Short boolean flag, e.g. -v / -h. Everything after the single
            // dash is the flag name (no bundling, no value).
            flags.insert(arg[1..].to_string(), FlagValue::Bool);
            i += 1;
        } else {
            positionals.push(arg.clone());
            i += 1;
        }
    }

    ParsedArgs {
        command,
        positionals,
        flags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_the_command_positionals_and_flags() {
        let a = parse_args(&args(&[
            "start",
            "add greet",
            "--profile",
            "bugfix",
            "--json",
        ]));
        assert_eq!(a.command.as_deref(), Some("start"));
        assert_eq!(a.positionals, vec!["add greet".to_string()]);
        assert_eq!(a.flags.str("profile"), Some("bugfix"));
        assert!(a.flags.is_true("json"));
    }

    #[test]
    fn supports_flag_equals_value() {
        let a = parse_args(&args(&["check", "--format=toon"]));
        assert_eq!(a.flags.str("format"), Some("toon"));
    }

    #[test]
    fn treats_a_leading_long_flag_as_no_command() {
        let a = parse_args(&args(&["--version"]));
        assert_eq!(a.command, None);
        assert!(a.flags.is_true("version"));
    }

    #[test]
    fn treats_a_leading_short_flag_as_no_command() {
        let a = parse_args(&args(&["-v"]));
        assert_eq!(a.command, None);
        assert!(a.flags.is_true("v"));
        let a2 = parse_args(&args(&["-h"]));
        assert_eq!(a2.command, None);
        assert!(a2.flags.is_true("h"));
    }

    #[test]
    fn returns_a_null_command_for_empty_argv() {
        assert_eq!(parse_args(&args(&[])).command, None);
    }

    #[test]
    fn a_flags_value_is_only_withheld_when_the_next_token_starts_with_double_dash() {
        // Quirk ported verbatim from src/cli/args.ts: a single-dash next
        // token (e.g. "-1") IS swallowed as the value.
        let a = parse_args(&args(&["--profile", "-x"]));
        assert_eq!(a.flags.str("profile"), Some("-x"));
    }

    #[test]
    fn a_flag_at_the_end_of_argv_with_no_following_token_becomes_boolean_true() {
        let a = parse_args(&args(&["--refresh"]));
        assert!(a.flags.is_true("refresh"));
    }

    #[test]
    fn an_eq_form_value_is_always_a_string_even_when_it_looks_boolean() {
        let a = parse_args(&args(&["--json=false"]));
        assert_eq!(a.flags.str("json"), Some("false"));
        assert!(!a.flags.is_true("json"));
    }
}
