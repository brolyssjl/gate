//! Adapted from `src/core/version.ts`: the CLI version string.
//!
//! TS's `readVersion()` walks up from the module's own location to the
//! nearest `package.json` at runtime, falling back to a build-time embedded
//! constant when none is reachable (a single-file binary ships with no
//! sibling `package.json`). The Rust binary has no runtime package
//! manifest at all, so this always returns the crate's own compiled-in
//! version, `Cargo.toml`'s `version` field via `env!("CARGO_PKG_VERSION")`.
//! The release workflow keeps that field in lockstep with `package.json`
//! and the release tag (see `docs/rust-port.md`'s exactness contract).

pub fn read_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_cargo_toml_and_stays_in_lockstep_with_package_json() {
        let package_json =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../package.json"))
                .unwrap();
        let pkg_version = package_json
            .lines()
            .find_map(|l| l.trim().strip_prefix("\"version\": \""))
            .and_then(|rest| rest.split('"').next())
            .unwrap();
        assert_eq!(read_version(), pkg_version);
    }

    #[test]
    fn is_non_empty_and_has_no_surrounding_whitespace() {
        let v = read_version();
        assert!(!v.is_empty());
        assert_eq!(v.trim(), v);
    }
}
