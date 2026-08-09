//! Full `aam skills adopt` (`docs/09-skills-management.md` §9.6): the
//! Phase 3 extension of Phase 1's `share_skill_with_codex`-only version.
//! Two independent flows:
//!
//! - [`adopt_local_skill`]: the skill is already somewhere on this
//!   machine (canonical store already, or a location `discover.rs`'s
//!   `scan_unmanaged_skills` reported) -- move its content into the
//!   canonical store if it isn't there already, transactionally
//!   ([`AdoptSkillMove`]).
//! - [`adopt_from_git`]: the skill doesn't exist locally yet -- clone it
//!   straight from a git URL into the canonical store.
//!
//! Both record the result in [`crate::SkillsIndex`]; `--share-with`
//! itself stays the CLI layer's job (unchanged from Phase 1's
//! `share_skill_with_codex`), recorded via
//! [`crate::SkillsIndex::record_share_target`].

use crate::index::{IndexError, SkillEntry, SkillsIndex};
use crate::link::{self, LinkError, ProvisionDirLink};
use crate::paths::claude_personal_skills_dir;
use aam_core::{execute, ExecuteError, TransactionalOp};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum AdoptMoveError {
    /// `source_path` has no `SKILL.md` -- not a skill at all.
    NotASkill(PathBuf),
    /// Something already exists at the canonical destination -- refuses
    /// to clobber it (could be an unrelated skill that happens to share
    /// this name).
    CanonicalAlreadyExists(PathBuf),
    Io(io::Error),
    Link(ExecuteError<LinkError>),
    VerifyFailed(String),
}

impl fmt::Display for AdoptMoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdoptMoveError::NotASkill(p) => write!(f, "{} has no SKILL.md -- not a skill", p.display()),
            AdoptMoveError::CanonicalAlreadyExists(p) => {
                write!(f, "{} already exists -- refusing to overwrite", p.display())
            }
            AdoptMoveError::Io(e) => write!(f, "I/O error: {e}"),
            AdoptMoveError::Link(e) => write!(f, "{e}"),
            AdoptMoveError::VerifyFailed(msg) => write!(f, "{msg}"),
        }
    }
}

impl Error for AdoptMoveError {}

impl From<io::Error> for AdoptMoveError {
    fn from(e: io::Error) -> Self {
        AdoptMoveError::Io(e)
    }
}

/// Moves `source_path`'s content to `canonical_path`, then links
/// `source_path` back to it (`ProvisionDirLink`) so anything that used to
/// point at the old location keeps working -- an `aam_core::TransactionalOp`
/// so a failure partway through (e.g. the link step) puts the content
/// back where it started rather than leaving it stranded at the
/// destination with a broken source.
pub struct AdoptSkillMove {
    source_path: PathBuf,
    canonical_path: PathBuf,
    moved: bool,
}

impl AdoptSkillMove {
    pub fn new(source_path: impl Into<PathBuf>, canonical_path: impl Into<PathBuf>) -> Self {
        Self {
            source_path: source_path.into(),
            canonical_path: canonical_path.into(),
            moved: false,
        }
    }
}

impl TransactionalOp for AdoptSkillMove {
    type Snapshot = ();
    type Error = AdoptMoveError;

    fn snapshot(&self) -> Result<(), AdoptMoveError> {
        if !self.source_path.join("SKILL.md").is_file() {
            return Err(AdoptMoveError::NotASkill(self.source_path.clone()));
        }
        if self.canonical_path.exists() {
            return Err(AdoptMoveError::CanonicalAlreadyExists(self.canonical_path.clone()));
        }
        Ok(())
    }

    fn apply(&mut self) -> Result<(), AdoptMoveError> {
        // `fs::rename` requires the destination's parent to already
        // exist -- the canonical skills root or a not-yet-created nested
        // parent otherwise makes this fail with "path not found" before
        // ever getting to the interesting part.
        if let Some(parent) = self.canonical_path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Same-volume only: cross-volume `rename` fails outright on
        // Windows, and this round deliberately doesn't implement a
        // recursive copy+delete fallback for that uncommon case (08).
        fs::rename(&self.source_path, &self.canonical_path)?;
        self.moved = true;

        let mut link_op = ProvisionDirLink::new(self.source_path.clone(), self.canonical_path.clone());
        execute(&mut link_op).map_err(AdoptMoveError::Link)?;
        Ok(())
    }

    fn verify(&self) -> Result<(), AdoptMoveError> {
        if !self.canonical_path.join("SKILL.md").is_file() {
            return Err(AdoptMoveError::VerifyFailed(format!(
                "{} has no SKILL.md after move",
                self.canonical_path.display()
            )));
        }
        if !link::resolves_to(&self.source_path, &self.canonical_path) {
            return Err(AdoptMoveError::VerifyFailed(format!(
                "{} does not resolve back to {}",
                self.source_path.display(),
                self.canonical_path.display()
            )));
        }
        Ok(())
    }

    fn rollback(&mut self, _snapshot: ()) -> Result<(), AdoptMoveError> {
        if !self.moved {
            return Ok(());
        }
        // If the link step ran and failed, its own rollback (inside the
        // nested `execute` call in `apply`) already removed whatever it
        // created at `source_path`, leaving it absent. Clear it
        // defensively in case something else is there before moving the
        // content back -- `remove_dir` only ever succeeds on an empty
        // directory or a link/reparse point, never on real content, so
        // this can't silently destroy anything unexpected.
        if self.source_path.exists() {
            fs::remove_dir(&self.source_path)?;
        }
        fs::rename(&self.canonical_path, &self.source_path)?;
        self.moved = false;
        Ok(())
    }
}

#[derive(Debug)]
pub enum AdoptError {
    /// `name` isn't at the canonical location and wasn't found in any of
    /// the given search directories either.
    NotFound(String),
    AlreadyExists(PathBuf),
    Move(Box<ExecuteError<AdoptMoveError>>),
    Index(IndexError),
    /// `git` isn't on PATH.
    GitNotFound,
    GitFailed(String),
    /// `git clone` succeeded but the result has no `SKILL.md` -- not a
    /// skill repo (or the skill lives in a subdirectory, which this round
    /// doesn't support -- see `08`'s sparse-checkout note).
    NotASkillRepo(String),
    Io(io::Error),
}

impl fmt::Display for AdoptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdoptError::NotFound(name) => write!(
                f,
                "no skill named '{name}' found at the canonical location or in any scanned directory \
                 -- run `aam skills scan` first"
            ),
            AdoptError::AlreadyExists(p) => write!(f, "{} already exists", p.display()),
            AdoptError::Move(e) => write!(f, "{e}"),
            AdoptError::Index(e) => write!(f, "{e}"),
            AdoptError::GitNotFound => write!(f, "`git` was not found on PATH"),
            AdoptError::GitFailed(msg) => write!(f, "git failed: {msg}"),
            AdoptError::NotASkillRepo(url) => {
                write!(f, "{url} was cloned but has no SKILL.md at its root")
            }
            AdoptError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl Error for AdoptError {}

impl From<IndexError> for AdoptError {
    fn from(e: IndexError) -> Self {
        AdoptError::Index(e)
    }
}
impl From<io::Error> for AdoptError {
    fn from(e: io::Error) -> Self {
        AdoptError::Io(e)
    }
}

/// [`adopt_local_skill`], parameterized on the canonical root -- lets
/// tests target a throwaway directory instead of the real
/// `~/.claude/skills` (`claude_personal_skills_dir()` has no env-var
/// override the way `aam_core::aam_home()` does), same reasoning as
/// `manage.rs`'s `install_bundled_skill`/`install_bundled_skill_at` split.
pub fn adopt_local_skill_at(
    canonical_root: &std::path::Path,
    index: &SkillsIndex,
    name: &str,
    search_dirs: &[(String, PathBuf)],
) -> Result<(), AdoptError> {
    let canonical_path = canonical_root.join(name);

    if !canonical_path.join("SKILL.md").is_file() {
        let source_path = search_dirs
            .iter()
            .map(|(_, dir)| dir.join(name))
            .find(|p| p.join("SKILL.md").is_file())
            .ok_or_else(|| AdoptError::NotFound(name.to_string()))?;

        let mut op = AdoptSkillMove::new(source_path, canonical_path);
        execute(&mut op).map_err(|e| AdoptError::Move(Box::new(e)))?;
    }

    let mut entry = index.get(name)?.unwrap_or_else(|| SkillEntry::new_local(name));
    entry.managed = true;
    index.upsert(entry)?;
    Ok(())
}

/// Thin wrapper over [`adopt_local_skill_at`] rooted at the real
/// `~/.claude/skills` -- what `aam-cli` actually calls.
pub fn adopt_local_skill(
    index: &SkillsIndex,
    name: &str,
    search_dirs: &[(String, PathBuf)],
) -> Result<(), AdoptError> {
    adopt_local_skill_at(&claude_personal_skills_dir(), index, name, search_dirs)
}

/// [`adopt_from_git`], parameterized on the canonical root -- see
/// [`adopt_local_skill_at`]'s doc comment for why.
pub fn adopt_from_git_at(
    canonical_root: &std::path::Path,
    index: &SkillsIndex,
    name: &str,
    url: &str,
    git_ref: Option<&str>,
    update_mode: &str,
) -> Result<(), AdoptError> {
    let canonical_path = canonical_root.join(name);
    if canonical_path.exists() {
        return Err(AdoptError::AlreadyExists(canonical_path));
    }
    if let Some(parent) = canonical_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut cmd = std::process::Command::new("git");
    // See updates.rs's `run_git` for why: avoids git's "dubious
    // ownership" guard tripping when the canonical skills dir's owner
    // doesn't match the running account (observed on CI runners).
    cmd.arg("-c").arg("safe.directory=*");
    cmd.arg("clone").arg("--depth").arg("1").arg("--quiet");
    if let Some(r) = git_ref {
        cmd.arg("--branch").arg(r);
    }
    cmd.arg(url).arg(&canonical_path);

    let status = cmd.status().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            AdoptError::GitNotFound
        } else {
            AdoptError::Io(e)
        }
    })?;
    if !status.success() {
        return Err(AdoptError::GitFailed(format!("git clone exited with {status}")));
    }
    if !canonical_path.join("SKILL.md").is_file() {
        return Err(AdoptError::NotASkillRepo(url.to_string()));
    }

    let source = match git_ref {
        Some(r) => format!("{url}@{r}"),
        None => url.to_string(),
    };
    let mut entry = SkillEntry::new_local(name);
    entry.source = source;
    entry.update_mode = update_mode.to_string();
    index.upsert(entry)?;
    Ok(())
}

/// `aam skills adopt <name> --source <url>[@ref]`: thin wrapper over
/// [`adopt_from_git_at`] rooted at the real `~/.claude/skills`.
pub fn adopt_from_git(
    index: &SkillsIndex,
    name: &str,
    url: &str,
    git_ref: Option<&str>,
    update_mode: &str,
) -> Result<(), AdoptError> {
    adopt_from_git_at(&claude_personal_skills_dir(), index, name, url, git_ref, update_mode)
}

/// Splits `--source`'s `<url>[@ref]` syntax (`09.5`). Uses the *last* `@`
/// and only treats what follows it as a ref if that part contains neither
/// `/` nor `:` -- SSH-style URLs (`git@github.com:user/repo.git`) already
/// have an `@` in the host, and naively splitting on the first `@` would
/// mistake `github.com:user/repo.git` for a ref name.
pub fn parse_source_spec(spec: &str) -> (String, Option<String>) {
    if let Some((url, git_ref)) = spec.rsplit_once('@') {
        if !git_ref.is_empty() && !git_ref.contains('/') && !git_ref.contains(':') {
            return (url.to_string(), Some(git_ref.to_string()));
        }
    }
    (spec.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "aam-skills-adopt-test-{label}-{}-{unique}",
                std::process::id()
            ));
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn make_skill(dir: &std::path::Path) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("SKILL.md"), "---\nname: x\ndescription: x\n---\n").unwrap();
        fs::write(dir.join("marker.txt"), b"original content").unwrap();
    }

    #[test]
    fn moves_content_and_links_back() {
        let base = TempDir::new("move-basic");
        let source = base.0.join("source").join("my-skill");
        make_skill(&source);
        let canonical = base.0.join("canonical").join("my-skill");

        let mut op = AdoptSkillMove::new(&source, &canonical);
        execute(&mut op).expect("move should succeed");

        assert!(canonical.join("marker.txt").is_file());
        assert_eq!(fs::read_to_string(canonical.join("marker.txt")).unwrap(), "original content");
        assert!(link::resolves_to(&source, &canonical));
    }

    #[test]
    fn refuses_when_canonical_destination_already_exists() {
        let base = TempDir::new("conflict");
        let source = base.0.join("source").join("my-skill");
        make_skill(&source);
        let canonical = base.0.join("canonical").join("my-skill");
        make_skill(&canonical);

        let mut op = AdoptSkillMove::new(&source, &canonical);
        let result = execute(&mut op);
        assert!(result.is_err());
        // Source content must be untouched.
        assert!(source.join("marker.txt").is_file());
    }

    #[test]
    fn rejects_a_source_without_skill_md() {
        let base = TempDir::new("not-a-skill");
        let source = base.0.join("source").join("not-a-skill");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("readme.txt"), b"nope").unwrap();
        let canonical = base.0.join("canonical").join("not-a-skill");

        let mut op = AdoptSkillMove::new(&source, &canonical);
        assert!(execute(&mut op).is_err());
        assert!(!canonical.exists());
    }

    #[test]
    fn rollback_restores_original_location() {
        let base = TempDir::new("rollback");
        let source = base.0.join("source").join("my-skill");
        make_skill(&source);
        let canonical = base.0.join("canonical").join("my-skill");

        let mut op = AdoptSkillMove::new(&source, &canonical);
        op.apply().expect("apply should succeed");
        assert!(canonical.join("marker.txt").is_file());
        assert!(!source.exists() || link::resolves_to(&source, &canonical));

        op.rollback(()).expect("rollback should succeed");
        assert!(source.join("marker.txt").is_file(), "content should be back at the source");
        assert_eq!(fs::read_to_string(source.join("marker.txt")).unwrap(), "original content");
        assert!(!canonical.exists(), "canonical location should be empty again after rollback");
    }

    #[test]
    fn adopt_local_skill_at_records_already_canonical_skill_without_moving() {
        let base = TempDir::new("already-canonical");
        let canonical_root = base.0.join("canonical");
        make_skill(&canonical_root.join("my-skill"));
        let index = SkillsIndex::open(base.0.join(".aam-skills-index.json"));

        adopt_local_skill_at(&canonical_root, &index, "my-skill", &[]).unwrap();

        let entry = index.get("my-skill").unwrap().unwrap();
        assert!(entry.managed);
        assert_eq!(entry.source, "local");
        // Nothing should have been moved -- it was already canonical.
        assert!(canonical_root.join("my-skill/marker.txt").is_file());
    }

    #[test]
    fn adopt_local_skill_at_finds_and_moves_from_a_search_dir() {
        let base = TempDir::new("search-dir");
        let canonical_root = base.0.join("canonical");
        let codex_dir = base.0.join("codex-skills");
        make_skill(&codex_dir.join("my-skill"));
        let index = SkillsIndex::open(base.0.join(".aam-skills-index.json"));

        adopt_local_skill_at(
            &canonical_root,
            &index,
            "my-skill",
            &[("codex".to_string(), codex_dir.clone())],
        )
        .unwrap();

        assert!(canonical_root.join("my-skill/marker.txt").is_file());
        assert!(link::resolves_to(&codex_dir.join("my-skill"), &canonical_root.join("my-skill")));
        assert!(index.get("my-skill").unwrap().unwrap().managed);
    }

    #[test]
    fn adopt_local_skill_at_errors_when_not_found_anywhere() {
        let base = TempDir::new("not-found");
        let index = SkillsIndex::open(base.0.join(".aam-skills-index.json"));
        let err = adopt_local_skill_at(&base.0.join("canonical"), &index, "ghost", &[]).unwrap_err();
        assert!(matches!(err, AdoptError::NotFound(_)));
    }

    #[test]
    fn parse_source_spec_plain_url_has_no_ref() {
        assert_eq!(
            parse_source_spec("https://github.com/user/repo.git"),
            ("https://github.com/user/repo.git".to_string(), None)
        );
    }

    #[test]
    fn parse_source_spec_splits_off_a_trailing_ref() {
        assert_eq!(
            parse_source_spec("https://github.com/user/repo.git@v1.2.3"),
            ("https://github.com/user/repo.git".to_string(), Some("v1.2.3".to_string()))
        );
    }

    #[test]
    fn parse_source_spec_does_not_mistake_an_ssh_host_for_a_ref() {
        // `git@github.com:user/repo.git` has an `@` in the host, not
        // separating a ref -- the part after the last `@` here is
        // `github.com:user/repo.git`, which contains both `/` and `:`,
        // so it must be treated as (no ref), not misparsed.
        assert_eq!(
            parse_source_spec("git@github.com:user/repo.git"),
            ("git@github.com:user/repo.git".to_string(), None)
        );
    }

    #[test]
    fn parse_source_spec_ssh_url_with_explicit_ref_still_splits_correctly() {
        assert_eq!(
            parse_source_spec("git@github.com:user/repo.git@main"),
            ("git@github.com:user/repo.git".to_string(), Some("main".to_string()))
        );
    }
}
