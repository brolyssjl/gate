//! The CLI version string: the crate's own compiled-in version,
//! `Cargo.toml`'s `version` field via `env!("CARGO_PKG_VERSION")`. The
//! release workflow's version guard keeps that field in lockstep with the
//! release tag.

pub fn read_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_non_empty_and_has_no_surrounding_whitespace() {
        let v = read_version();
        assert!(!v.is_empty());
        assert_eq!(v.trim(), v);
    }
}
