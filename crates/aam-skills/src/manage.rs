use crate::link::{self, LinkError, ProvisionDirLink};
use crate::paths::{claude_personal_skills_dir, codex_user_skills_dir};
use aam_core::{execute, ExecuteError};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum ShareError {
    Link(ExecuteError<LinkError>),
    /// No `~/.claude/skills/<name>/SKILL.md` exists -- this Phase 1
    /// `share_skill_with_codex` only links an already-canonical skill; it
    /// does not move content from elsewhere (that's Phase 3's fuller
    /// `adopt`, see `docs/09-skills-management.md` §9.6).
    SkillNotFound(String),
    Io(io::Error),
}

impl fmt::Display for ShareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShareError::Link(e) => write!(f, "{e}"),
            ShareError::SkillNotFound(name) => write!(
                f,
                "no skill named '{name}' found at {}/{name} (missing SKILL.md)",
                claude_personal_skills_dir().display()
            ),
            ShareError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl Error for ShareError {}

impl From<io::Error> for ShareError {
    fn from(e: io::Error) -> Self {
        ShareError::Io(e)
    }
}

/// Provisions `<profile_dir>/skills` to resolve to the canonical
/// `~/.claude/skills` store, so a Claude Profile never has its own
/// diverging copy (`docs/03-credential-account-module.md` §3.7).
pub fn provision_profile_skills_link(profile_dir: &std::path::Path) -> Result<(), ExecuteError<LinkError>> {
    let link_path = profile_dir.join("skills");
    let target = claude_personal_skills_dir();
    let mut op = ProvisionDirLink::new(link_path, target);
    execute(&mut op)
}

/// Explicitly shares an already-canonical skill with Codex by linking it
/// into `$HOME/.agents/skills/<name>`. Returns the list of SKILL.md
/// frontmatter keys beyond `name`/`description` found on the skill, if
/// any -- callers should surface these as a compatibility warning
/// (`docs/09-skills-management.md` §9.3: "结构兼容，高级特性不保证互通"),
/// not treat them as fatal.
pub fn share_skill_with_codex(name: &str) -> Result<Vec<String>, ShareError> {
    let canonical = claude_personal_skills_dir().join(name);
    if !canonical.join("SKILL.md").is_file() {
        return Err(ShareError::SkillNotFound(name.to_string()));
    }

    let link_path = codex_user_skills_dir().join(name);
    let mut op = ProvisionDirLink::new(link_path, canonical.clone());
    execute(&mut op).map_err(ShareError::Link)?;

    Ok(non_standard_frontmatter_keys(&canonical.join("SKILL.md"))?)
}

/// Parses just enough of a `SKILL.md`'s YAML frontmatter to list its
/// top-level keys -- a hand-rolled scan rather than a full YAML parser,
/// since all we need is "which keys exist", not their values, and pulling
/// in a YAML dependency for that would be overkill for Phase 1.
fn non_standard_frontmatter_keys(skill_md_path: &std::path::Path) -> io::Result<Vec<String>> {
    let text = fs::read_to_string(skill_md_path)?;
    let mut lines = text.lines();

    if lines.next() != Some("---") {
        return Ok(Vec::new());
    }

    let mut extra_keys = Vec::new();
    for line in lines {
        if line.trim_end() == "---" {
            break;
        }
        // Only top-level (unindented) `key:` lines count as frontmatter
        // fields; indented lines are nested values/continuations.
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        if let Some((key, _)) = line.split_once(':') {
            let key = key.trim();
            if !key.is_empty() && key != "name" && key != "description" {
                extra_keys.push(key.to_string());
            }
        }
    }
    Ok(extra_keys)
}

/// A skill known to the canonical `~/.claude/skills` store, with its
/// current link status -- the read model behind `aam skills list/status`.
#[derive(Debug)]
pub struct ManagedSkill {
    pub name: String,
    pub canonical_path: PathBuf,
    pub linked_to_codex: bool,
    /// Whether the canonical store (or this skill's own directory) looks
    /// like a git repository -- if so, `aam skills status` should suggest
    /// `git push`/`git pull` rather than anything else (`09.2`).
    pub is_git_repo: bool,
}

/// Lists every skill currently in the canonical store
/// (`~/.claude/skills/<name>/SKILL.md`), with cross-tool link status.
pub fn list_managed_skills() -> io::Result<Vec<ManagedSkill>> {
    let root = claude_personal_skills_dir();
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let root_is_git = root.join(".git").is_dir();
    let mut out = Vec::new();

    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() || !path.join("SKILL.md").is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let codex_link = codex_user_skills_dir().join(&name);
        out.push(ManagedSkill {
            linked_to_codex: link::resolves_to(&codex_link, &path),
            is_git_repo: root_is_git || path.join(".git").is_dir(),
            canonical_path: path,
            name,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_keys_beyond_name_and_description() {
        let dir = std::env::temp_dir().join(format!(
            "aam-skills-frontmatter-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: demo\ndescription: a demo skill\nallowed-tools: bash\n---\n\n# Demo\n",
        )
        .unwrap();

        let extra = non_standard_frontmatter_keys(&path).unwrap();
        assert_eq!(extra, vec!["allowed-tools".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_extra_keys_when_only_name_and_description_present() {
        let dir = std::env::temp_dir().join(format!(
            "aam-skills-frontmatter-test-plain-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");
        std::fs::write(&path, "---\nname: demo\ndescription: a demo skill\n---\n\n# Demo\n").unwrap();

        let extra = non_standard_frontmatter_keys(&path).unwrap();
        assert!(extra.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
