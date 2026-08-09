//! Skill discovery (`docs/09-skills-management.md` §9.6): finds skill
//! directories (anything with a `SKILL.md`) under a caller-supplied list
//! of locations that aren't already tracked in the [`crate::SkillsIndex`]
//! or already a link back to the canonical store.
//!
//! `aam-skills` depends on nothing but `aam-core` (`02.1`'s boundary), so
//! it has no way to ask `aam-switcher::ProfileRegistry` which Claude
//! Profiles exist -- callers (`aam-cli`) pass in the directories to search
//! as `(location_label, dir)` pairs, the same dependency-injection
//! pattern `provider_sync`/`account_sync` use for the domain knowledge
//! `aam-sync` itself isn't allowed to have.
//!
//! **Read-only**: never writes to the index -- that's `adopt.rs`'s job,
//! symmetric with `aam-memory`'s scan-then-adopt two-stage flow (`05.7`).

use crate::link::resolves_to;
use std::fs;
use std::path::{Path, PathBuf};

/// A skill found on disk that isn't in the index yet (or is in the index
/// but its on-disk location no longer matches -- currently `aam-skills`
/// doesn't try to detect that case, only "never seen before").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSkill {
    pub name: String,
    /// Where it was found, e.g. `"claude-canonical"`, `"codex"`, or
    /// whatever label the caller gave that search directory (e.g. a
    /// Profile's own label).
    pub location: String,
    pub path: PathBuf,
}

/// Scans every `(location_label, dir)` pair's immediate children for
/// skill directories (`<dir>/<name>/SKILL.md`), skipping:
/// - anything already in `index` (by name -- already known, regardless of
///   whether it's still `managed` or just previously discovered),
/// - anything that's already a link resolving back to the canonical store
///   (`canonical_root`) -- that's already effectively managed, just not
///   necessarily recorded in the index yet (e.g. Phase 1's automatic
///   per-Profile links).
pub fn scan_unmanaged_skills(
    known_names: &[String],
    canonical_root: &Path,
    search_dirs: &[(String, PathBuf)],
) -> std::io::Result<Vec<DiscoveredSkill>> {
    let mut out = Vec::new();

    for (location, dir) in search_dirs {
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() || !path.join("SKILL.md").is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue; // e.g. a stray dotfile-ish directory, not a real skill
            }
            if known_names.iter().any(|n| n == &name) {
                continue;
            }
            let canonical_target = canonical_root.join(&name);
            if resolves_to(&path, &canonical_target) {
                continue; // already linked back to canonical, effectively managed
            }
            out.push(DiscoveredSkill {
                name,
                location: location.clone(),
                path,
            });
        }
    }

    out.sort_by(|a, b| (&a.name, &a.location).cmp(&(&b.name, &b.location)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::ProvisionDirLink;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "aam-skills-discover-test-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn make_skill(dir: &Path, name: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "---\nname: x\ndescription: x\n---\n").unwrap();
    }

    #[test]
    fn discovers_a_skill_in_a_search_dir() {
        let base = TempDir::new("basic");
        let canonical = base.0.join("canonical");
        fs::create_dir_all(&canonical).unwrap();
        let codex_dir = base.0.join("codex-skills");
        fs::create_dir_all(&codex_dir).unwrap();
        make_skill(&codex_dir, "some-skill");

        let found = scan_unmanaged_skills(&[], &canonical, &[("codex".to_string(), codex_dir)]).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "some-skill");
        assert_eq!(found[0].location, "codex");
    }

    #[test]
    fn skips_names_already_in_the_index() {
        let base = TempDir::new("known");
        let canonical = base.0.join("canonical");
        fs::create_dir_all(&canonical).unwrap();
        let codex_dir = base.0.join("codex-skills");
        fs::create_dir_all(&codex_dir).unwrap();
        make_skill(&codex_dir, "already-known");

        let found = scan_unmanaged_skills(
            &["already-known".to_string()],
            &canonical,
            &[("codex".to_string(), codex_dir)],
        )
        .unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn skips_items_already_linked_back_to_canonical() {
        let base = TempDir::new("linked");
        let canonical = base.0.join("canonical");
        fs::create_dir_all(&canonical).unwrap();
        make_skill(&canonical, "shared-skill");

        let codex_dir = base.0.join("codex-skills");
        let mut op = ProvisionDirLink::new(codex_dir.join("shared-skill"), canonical.join("shared-skill"));
        aam_core::execute(&mut op).unwrap();

        let found = scan_unmanaged_skills(&[], &canonical, &[("codex".to_string(), codex_dir)]).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn ignores_non_skill_directories_and_missing_search_dirs() {
        let base = TempDir::new("misc");
        let canonical = base.0.join("canonical");
        fs::create_dir_all(&canonical).unwrap();
        let codex_dir = base.0.join("codex-skills");
        fs::create_dir_all(codex_dir.join("not-a-skill")).unwrap(); // no SKILL.md

        let found = scan_unmanaged_skills(
            &[],
            &canonical,
            &[
                ("codex".to_string(), codex_dir),
                ("missing".to_string(), base.0.join("does-not-exist")),
            ],
        )
        .unwrap();
        assert!(found.is_empty());
    }
}
