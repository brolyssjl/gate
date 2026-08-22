//! Port of `src/integrations/index.ts` (`detect`, `sddDir`, `planHints` -
//! advisory-only integration detection). Ported in wave 3 (see
//! `docs/rust-port.md`).
//!
//! One module per `src/integrations/*.ts` file, same names.
//!
//! Integrations are **advisory, never load-bearing** (kickoff principle 5):
//! detection only changes hint lines and config, and no gate check depends
//! on an external tool being installed. Detection is pure filesystem
//! presence, so `init --refresh` re-detects idempotently.

use std::path::Path;

pub mod advise;
pub mod agnosgram_write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sdd {
    OpenSpec,
    SpecKit,
    Bmad,
}

impl Sdd {
    pub fn as_str(&self) -> &'static str {
        match self {
            Sdd::OpenSpec => "openspec",
            Sdd::SpecKit => "spec-kit",
            Sdd::Bmad => "bmad",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection {
    pub agnosgram: bool,
    pub sdd: Option<Sdd>,
}

/// Directory name for each SDD framework `detect_sdd` recognizes, in
/// detection order.
const SDD_DIRS: &[(Sdd, &str)] = &[
    (Sdd::OpenSpec, "openspec"),
    (Sdd::SpecKit, ".specify"),
    (Sdd::Bmad, "_bmad"),
    (Sdd::Bmad, ".bmad"),
];

pub fn detect(root: &Path) -> Detection {
    Detection {
        agnosgram: root.join(".agnosgram").exists(),
        sdd: detect_sdd(root),
    }
}

fn detect_sdd(root: &Path) -> Option<Sdd> {
    SDD_DIRS
        .iter()
        .find(|(_, dir)| root.join(dir).exists())
        .map(|(kind, _)| *kind)
}

/// The repo-relative SDD directory Gate detected, or `None` if none. Used by
/// the PLAN gate to check a cited `spec:` path lives under it. Distinct from
/// `detect(root).sdd` (which names the *framework*) because `bmad` has two
/// possible directory names; this resolves to the one actually present.
pub fn sdd_dir(root: &Path) -> Option<&'static str> {
    SDD_DIRS
        .iter()
        .find(|(_, dir)| root.join(dir).exists())
        .map(|(_, dir)| *dir)
}

/// Hint lines appended to the PLAN playbook output when integrations are
/// present.
pub fn plan_hints(det: Detection) -> Vec<String> {
    let mut hints = Vec::new();
    if det.agnosgram {
        hints.push(
            "Agnosgram detected: read `.agnosgram/lessons/pitfalls.md` before planning."
                .to_string(),
        );
    }
    if let Some(sdd) = det.sdd {
        hints.push(format!(
            "SDD ({}) detected: cite the spec path in plan.md instead of restating it.",
            sdd.as_str()
        ));
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gate-integrations-rs-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_nothing_in_a_plain_repo() {
        let root = tmp_dir("plain");
        let det = detect(&root);
        assert!(!det.agnosgram);
        assert_eq!(det.sdd, None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn detects_agnosgram_and_openspec() {
        let root = tmp_dir("both");
        fs::create_dir_all(root.join(".agnosgram")).unwrap();
        fs::create_dir_all(root.join("openspec")).unwrap();
        let det = detect(&root);
        assert!(det.agnosgram);
        assert_eq!(det.sdd, Some(Sdd::OpenSpec));
        assert_eq!(sdd_dir(&root), Some("openspec"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bmad_resolves_to_whichever_of_its_two_directory_names_is_present() {
        let root = tmp_dir("bmad-underscore");
        fs::create_dir_all(root.join("_bmad")).unwrap();
        assert_eq!(detect(&root).sdd, Some(Sdd::Bmad));
        assert_eq!(sdd_dir(&root), Some("_bmad"));
        fs::remove_dir_all(&root).unwrap();

        let root2 = tmp_dir("bmad-dot");
        fs::create_dir_all(root2.join(".bmad")).unwrap();
        assert_eq!(detect(&root2).sdd, Some(Sdd::Bmad));
        assert_eq!(sdd_dir(&root2), Some(".bmad"));
        fs::remove_dir_all(&root2).unwrap();
    }

    #[test]
    fn plan_hints_includes_a_line_per_detected_integration() {
        let hints = plan_hints(Detection {
            agnosgram: true,
            sdd: Some(Sdd::OpenSpec),
        });
        assert_eq!(hints.len(), 2);
        assert!(hints[0].contains("Agnosgram"));
        assert!(hints[1].contains("openspec"));
    }

    #[test]
    fn plan_hints_is_empty_with_nothing_detected() {
        assert!(plan_hints(Detection {
            agnosgram: false,
            sdd: None
        })
        .is_empty());
    }
}
