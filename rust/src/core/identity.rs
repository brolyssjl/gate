//! New (not a TS port): identity resolution for the acting party recorded
//! by every identity-carrying command - `gate trust`/`gate approve`/`gate
//! streak reset` (`trustedBy`, `approval.by`, `override.by`; issue #29)
//! plus `gate skip` (`override.by`), `gate amend` (`amendment.by`), `gate
//! review`'s packet request (`review.requestedBy`), and `gate start`'s
//! audit-only `startedBy` (issue #33) - see README's "Identity fallback"
//! section. Every human checkpoint used to record `by: null` unless the
//! caller happened to pass `--by` or set `GATE_SESSION_ID`; nothing
//! nudged toward either, so the audit trail said *that* a checkpoint was
//! passed but not *who* passed it.
//!
//! This is deliberately weak, spoofable identity - `git config user.name`
//! is whatever the local config says, not a proof of anything. It's still
//! strictly better than `null` for the common case (see the README's
//! threat model: Gate defends against sloppiness, not malice). Real,
//! unspoofable session identity is a bigger effort tracked separately
//! (ROADMAP.md's "Reviewer identity threading").

use std::path::Path;

use crate::cli::args::ParsedArgs;
use crate::cli::output::UserError;
use crate::core::config::GateConfig;
use crate::core::git;

/// Resolve who's acting: `--by` > `GATE_SESSION_ID` > `git config
/// user.name` (trimmed; empty or erroring treated as absent) > `None`.
pub fn resolve(args: &ParsedArgs, root: &Path) -> Option<String> {
    args.flags
        .str("by")
        .map(str::to_string)
        .or_else(|| std::env::var("GATE_SESSION_ID").ok())
        .or_else(|| git::user_name(root))
}

/// When `.gate/config.yml` sets `identity.require_identity: true`,
/// `resolve` coming back empty (all three sources absent) is a hard
/// failure rather than another silent `by: null` - the opt-in half of the
/// identity hardening in issue #29. Default is off: this never fires
/// unless a repo explicitly asks for it.
///
/// Deliberately does NOT check whether `by` differs from the identity
/// recorded on the run's IMPLEMENT-phase activity (i.e. no
/// non-implementer/self-approval enforcement) - that's issue #29's
/// proposal #2, explicitly deferred to the session-identity roadmap item
/// (ROADMAP.md's "Reviewer identity threading") because it needs a real,
/// unspoofable session identity to be meaningful rather than advisory.
pub fn require_if_configured(by: &Option<String>, config: &GateConfig) -> Result<(), UserError> {
    if by.is_none() && config.identity.require_identity {
        return Err(UserError::new(
            "no identity resolved for this action (config: identity.require_identity is true) - \
             pass --by <name>, set GATE_SESSION_ID, or set `git config user.name`",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::parse_args;
    use crate::core::config::{GateConfig, IdentityConfig};
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = crate::core::testutil::unique_temp_dir(&format!("identity-rs-{name}"));
        dir
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn make_repo(name: &str) -> std::path::PathBuf {
        let dir = tmp_dir(name);
        run_git(&dir, &["init", "-q"]);
        run_git(&dir, &["config", "user.email", "test@test.co"]);
        run_git(&dir, &["config", "user.name", "git-name"]);
        run_git(&dir, &["config", "commit.gpgsign", "false"]);
        dir
    }

    fn args(items: &[&str]) -> ParsedArgs {
        let mut full = vec!["cmd".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn by_flag_wins_over_everything() {
        let root = make_repo("by-flag-wins");
        let resolved = resolve(&args(&["--by", "alice"]), &root);
        assert_eq!(resolved.as_deref(), Some("alice"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn falls_back_to_git_user_name_when_by_and_env_are_absent() {
        let root = make_repo("git-fallback");
        // Deliberately not asserting anything about GATE_SESSION_ID here -
        // see the conformance suite for the env-precedence case, which can
        // isolate the environment per-subprocess.
        let resolved = resolve(&args(&[]), &root);
        assert_eq!(resolved.as_deref(), Some("git-name"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolves_to_none_outside_any_repo_with_no_flag_or_env() {
        let dir = tmp_dir("no-repo");
        // Not a git repo at all: `git config user.name` here still consults
        // global/system config, so this only pins the "no crash, and a real
        // Option<String> comes back either way" contract rather than a
        // specific None - the None case proper is covered end-to-end by the
        // conformance suite, which can isolate `$HOME`.
        let _ = resolve(&args(&[]), &dir);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn require_if_configured_passes_when_identity_present() {
        let config = GateConfig {
            identity: IdentityConfig {
                require_identity: true,
            },
            ..GateConfig::default()
        };
        assert!(require_if_configured(&Some("alice".to_string()), &config).is_ok());
    }

    #[test]
    fn require_if_configured_fails_when_absent_and_required() {
        let config = GateConfig {
            identity: IdentityConfig {
                require_identity: true,
            },
            ..GateConfig::default()
        };
        let err = require_if_configured(&None, &config).unwrap_err();
        assert_eq!(err.exit_code(), 1);
        assert!(err.message().contains("identity.require_identity"));
    }

    #[test]
    fn require_if_configured_is_a_noop_when_not_required() {
        let config = GateConfig::default();
        assert!(require_if_configured(&None, &config).is_ok());
    }
}
