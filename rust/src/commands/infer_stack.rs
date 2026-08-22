//! Port of `src/commands/inferStack.ts` (`inferCommands`,
//! `inferScopeIgnore`: stack detection for `gate init`'s generated
//! config.yml). Ported alongside `init.rs` (wave 4, see
//! `docs/rust-port.md`).

use std::fs;
use std::path::Path;

use crate::core::config::Commands;
use crate::core::json;

/// Best-effort command inference for common ecosystems. `gate init` writes
/// these as a starting point; anything unusual is hand-edited. Never fails -
/// an unknown stack simply yields empty commands with commented placeholders.
pub fn infer_commands(root: &Path) -> Commands {
    let pkg_path = root.join("package.json");
    if pkg_path.exists() {
        return infer_node(&pkg_path);
    }
    if root.join("pyproject.toml").exists() {
        return Commands {
            test: Some("pytest".to_string()),
            coverage: Some("pytest --cov --cov-report=json".to_string()),
            ..Default::default()
        };
    }
    if root.join("go.mod").exists() {
        return Commands {
            build: Some("go build ./...".to_string()),
            test: Some("go test ./...".to_string()),
            lint: Some("go vet ./...".to_string()),
            ..Default::default()
        };
    }
    if root.join("Cargo.toml").exists() {
        return Commands {
            build: Some("cargo build".to_string()),
            test: Some("cargo test".to_string()),
            lint: Some("cargo clippy".to_string()),
            ..Default::default()
        };
    }
    Commands::default()
}

fn infer_node(pkg_path: &Path) -> Commands {
    let scripts = fs::read_to_string(pkg_path)
        .ok()
        .and_then(|text| json::parse(&text).ok())
        .and_then(|v| v.get("scripts").cloned());
    let has = |name: &str| {
        scripts
            .as_ref()
            .and_then(|s| s.get(name))
            .and_then(|v| v.as_str())
            .is_some()
    };
    Commands {
        build: has("build").then(|| "npm run build".to_string()),
        test: has("test").then(|| "npm test".to_string()),
        lint: has("lint").then(|| "npm run lint".to_string()),
        coverage: has("coverage").then(|| "npm run coverage".to_string()),
    }
}

/// Best-effort `scope_ignore` seed per detected stack (Milestone 5): compile
/// caches and other environment cruft rewritten by the very commands Gate
/// runs, which would otherwise hard-block the scope check with false
/// violations (the motivating soak incident - see ROADMAP.md). Seeded once by
/// `gate init`, same as `infer_commands`; the human reviews and re-trusts
/// before it takes effect either way.
pub fn infer_scope_ignore(root: &Path) -> Vec<String> {
    let mut ignore = Vec::new();
    if root.join("package.json").exists() {
        ignore.extend(
            [
                "node_modules/**",
                ".cache/**",
                ".turbo/**",
                ".next/cache/**",
                ".nuxt/**",
            ]
            .map(String::from),
        );
    }
    if root.join("go.mod").exists() {
        ignore.extend(["bin/**", "vendor/**"].map(String::from));
    }
    if root.join("Cargo.toml").exists() {
        ignore.push("target/**".to_string());
    }
    if root.join("pyproject.toml").exists() {
        ignore.extend(["__pycache__/**", ".pytest_cache/**", ".mypy_cache/**"].map(String::from));
    }
    ignore.push("coverage/**".to_string());
    ignore
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-infer-stack-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn infers_node_commands_from_present_package_json_scripts() {
        let root = tmp_dir("node");
        fs::write(
            root.join("package.json"),
            r#"{"scripts": {"build": "tsc", "test": "vitest"}}"#,
        )
        .unwrap();
        let cmds = infer_commands(&root);
        assert_eq!(cmds.build.as_deref(), Some("npm run build"));
        assert_eq!(cmds.test.as_deref(), Some("npm test"));
        assert_eq!(cmds.lint, None);
        assert_eq!(cmds.coverage, None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tolerates_a_malformed_package_json() {
        let root = tmp_dir("node-malformed");
        fs::write(root.join("package.json"), "not json").unwrap();
        assert_eq!(infer_commands(&root), Commands::default());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn infers_python_commands_from_pyproject_toml() {
        let root = tmp_dir("python");
        fs::write(root.join("pyproject.toml"), "").unwrap();
        let cmds = infer_commands(&root);
        assert_eq!(cmds.test.as_deref(), Some("pytest"));
        assert_eq!(
            cmds.coverage.as_deref(),
            Some("pytest --cov --cov-report=json")
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn infers_go_commands_from_go_mod() {
        let root = tmp_dir("go");
        fs::write(root.join("go.mod"), "").unwrap();
        let cmds = infer_commands(&root);
        assert_eq!(cmds.build.as_deref(), Some("go build ./..."));
        assert_eq!(cmds.test.as_deref(), Some("go test ./..."));
        assert_eq!(cmds.lint.as_deref(), Some("go vet ./..."));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn infers_rust_commands_from_cargo_toml() {
        let root = tmp_dir("rust");
        fs::write(root.join("Cargo.toml"), "").unwrap();
        let cmds = infer_commands(&root);
        assert_eq!(cmds.build.as_deref(), Some("cargo build"));
        assert_eq!(cmds.test.as_deref(), Some("cargo test"));
        assert_eq!(cmds.lint.as_deref(), Some("cargo clippy"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn returns_empty_commands_for_an_unknown_stack() {
        let root = tmp_dir("unknown");
        assert_eq!(infer_commands(&root), Commands::default());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn seeds_scope_ignore_per_detected_stack_plus_the_universal_coverage_entry() {
        let root = tmp_dir("scope-ignore-node");
        fs::write(root.join("package.json"), "{}").unwrap();
        let ignore = infer_scope_ignore(&root);
        assert!(ignore.contains(&"node_modules/**".to_string()));
        assert!(ignore.contains(&"coverage/**".to_string()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scope_ignore_is_just_coverage_for_an_unknown_stack() {
        let root = tmp_dir("scope-ignore-unknown");
        assert_eq!(infer_scope_ignore(&root), vec!["coverage/**".to_string()]);
        fs::remove_dir_all(&root).unwrap();
    }
}
