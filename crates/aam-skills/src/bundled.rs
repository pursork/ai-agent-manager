//! Skills shipped with `aam` itself (`docs/09-skills-management.md`),
//! embedded into the binary at compile time via `include_str!` so `aam
//! skills install-bundled` doesn't depend on finding the source repo at
//! runtime -- works the same whether run from a checkout or a standalone
//! binary distribution.

/// One skill bundled with this binary: its canonical name, and every file
/// it needs as `(path relative to the skill's own directory, content)`
/// pairs.
pub struct BundledSkill {
    pub name: &'static str,
    pub files: &'static [(&'static str, &'static str)],
}

/// `project-tracker` (`docs/05-session-memory-bank-module.md` §5.1):
/// maintained in this repo going forward rather than only as a loose,
/// separately-authored skill, so it can stay in step with the
/// `deviceId`/`profileLabel`/`toolKind` fields `aam-memory` reads
/// (`08` #9's bridging decision -- `aam-memory` reads this same file).
pub const BUNDLED_SKILLS: &[BundledSkill] = &[BundledSkill {
    name: "project-tracker",
    files: &[
        ("SKILL.md", include_str!("../bundled/project-tracker/SKILL.md")),
        (
            "scripts/track-session.ps1",
            include_str!("../bundled/project-tracker/scripts/track-session.ps1"),
        ),
        (
            "scripts/backfill-index.ps1",
            include_str!("../bundled/project-tracker/scripts/backfill-index.ps1"),
        ),
    ],
}];

pub fn find_bundled_skill(name: &str) -> Option<&'static BundledSkill> {
    BUNDLED_SKILLS.iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_tracker_is_registered_and_non_empty() {
        let skill = find_bundled_skill("project-tracker").expect("should be registered");
        assert_eq!(skill.files.len(), 3);
        for (path, content) in skill.files {
            assert!(!content.is_empty(), "{path} should not be empty");
        }
    }

    #[test]
    fn unknown_name_returns_none() {
        assert!(find_bundled_skill("does-not-exist").is_none());
    }
}
