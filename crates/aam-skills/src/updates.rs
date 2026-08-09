//! GitHub-source update checking (`docs/09-skills-management.md` §9.7).
//! Only meaningful for skills adopted via `adopt_from_git` -- their
//! canonical directory *is* a git working tree (a real `git clone`), so
//! this doesn't maintain any extra "last known upstream commit" state of
//! its own; it just asks git.
//!
//! Not a real package manager (`00`'s Non-Goal, `09.7`): no dependency
//! resolution, no sparse-checkout for skills living in a subdirectory of
//! a larger repo (`docs/08-open-questions-risks.md`'s bandwidth note --
//! non-blocking future optimization, not done here).

use crate::index::{IndexError, SkillsIndex};
use crate::paths::claude_personal_skills_dir;
use std::error::Error;
use std::fmt;
use std::io;
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub enum UpdateError {
    Index(IndexError),
    GitNotFound,
    GitFailed(String),
    Io(io::Error),
    /// `name` isn't tracked, or its `source` is `"local"` -- nothing to
    /// check/update against.
    NotGitSourced(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::Index(e) => write!(f, "{e}"),
            UpdateError::GitNotFound => write!(f, "`git` was not found on PATH"),
            UpdateError::GitFailed(msg) => write!(f, "git failed: {msg}"),
            UpdateError::Io(e) => write!(f, "I/O error: {e}"),
            UpdateError::NotGitSourced(name) => {
                write!(f, "'{name}' has no git source recorded (source is \"local\" or it's not tracked)")
            }
        }
    }
}

impl Error for UpdateError {}

impl From<IndexError> for UpdateError {
    fn from(e: IndexError) -> Self {
        UpdateError::Index(e)
    }
}
impl From<io::Error> for UpdateError {
    fn from(e: io::Error) -> Self {
        UpdateError::Io(e)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateStatus {
    pub name: String,
    pub up_to_date: bool,
    pub local_commit: String,
    pub upstream_commit: String,
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, UpdateError> {
    let output = Command::new("git")
        // CI runners (and some locked-down corporate machines) can have
        // the canonical skills dir owned by a different account than the
        // one running `aam` -- git 2.35.2+'s "dubious ownership" guard
        // would otherwise refuse every command here. These directories
        // are aam-managed, not attacker-controlled, so trusting them
        // unconditionally is safe.
        .arg("-c")
        .arg("safe.directory=*")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                UpdateError::GitNotFound
            } else {
                UpdateError::Io(e)
            }
        })?;
    if !output.status.success() {
        return Err(UpdateError::GitFailed(format!(
            "git {} (in {}) exited with {}: {}",
            args.join(" "),
            dir.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn check_one(canonical_root: &Path, name: &str) -> Result<UpdateStatus, UpdateError> {
    let dir = canonical_root.join(name);
    run_git(&dir, &["fetch", "--quiet"])?;
    let local_commit = run_git(&dir, &["rev-parse", "HEAD"])?;
    let upstream_commit = run_git(&dir, &["rev-parse", "@{upstream}"])?;
    Ok(UpdateStatus {
        name: name.to_string(),
        up_to_date: local_commit == upstream_commit,
        local_commit,
        upstream_commit,
    })
}

/// [`check_updates`], parameterized on the canonical root (test seam,
/// same reasoning as `adopt.rs`'s `_at` functions).
pub fn check_updates_at(canonical_root: &Path, index: &SkillsIndex) -> Result<Vec<UpdateStatus>, UpdateError> {
    index
        .list()?
        .into_iter()
        .filter(|s| s.source != "local")
        .map(|s| check_one(canonical_root, &s.name))
        .collect()
}

pub fn check_updates(index: &SkillsIndex) -> Result<Vec<UpdateStatus>, UpdateError> {
    check_updates_at(&claude_personal_skills_dir(), index)
}

/// [`update_skill`], parameterized on the canonical root.
pub fn update_skill_at(canonical_root: &Path, index: &SkillsIndex, name: &str) -> Result<(), UpdateError> {
    let entry = index
        .get(name)?
        .filter(|e| e.source != "local")
        .ok_or_else(|| UpdateError::NotGitSourced(name.to_string()))?;

    let dir = canonical_root.join(&entry.name);
    run_git(&dir, &["fetch", "--quiet"])?;
    // These directories are meant to be pristine upstream mirrors, not
    // locally edited -- reset rather than merge, so there's never a
    // conflict to resolve (09.7: not a real package manager).
    run_git(&dir, &["reset", "--quiet", "--hard", "@{upstream}"])?;
    Ok(())
}

pub fn update_skill(index: &SkillsIndex, name: &str) -> Result<(), UpdateError> {
    update_skill_at(&claude_personal_skills_dir(), index, name)
}

/// `aam skills update --all-auto`: applies [`update_skill_at`] to every
/// One skill's outcome from [`update_all_auto`]/[`update_all_auto_at`] --
/// `Err` here means just that one skill's update failed, not the whole
/// batch (each entry is independent).
pub type AutoUpdateOutcome = (String, Result<(), UpdateError>);

/// entry with `update_mode == "auto"` (set via `adopt --update-mode
/// auto`). Still a command the user explicitly runs -- no background
/// timer -- matching `aam session approve-sync --all-scanned`'s pattern.
/// Returns one [`AutoUpdateOutcome`] per skill so one failure doesn't
/// stop the rest.
pub fn update_all_auto_at(
    canonical_root: &Path,
    index: &SkillsIndex,
) -> Result<Vec<AutoUpdateOutcome>, UpdateError> {
    let names: Vec<String> = index
        .list()?
        .into_iter()
        .filter(|s| s.source != "local" && s.update_mode == "auto")
        .map(|s| s.name)
        .collect();
    Ok(names
        .into_iter()
        .map(|name| {
            let result = update_skill_at(canonical_root, index, &name);
            (name, result)
        })
        .collect())
}

pub fn update_all_auto(index: &SkillsIndex) -> Result<Vec<AutoUpdateOutcome>, UpdateError> {
    update_all_auto_at(&claude_personal_skills_dir(), index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SkillEntry;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "aam-skills-updates-test-{label}-{}-{unique}",
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

    /// Runs `git` for test setup, panicking on failure -- these tests are
    /// skipped entirely (not run) in environments without `git` (checked
    /// once up front by `git_available()`), so a bare `git` invocation
    /// here is fine to assume works.
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-c")
            .arg("safe.directory=*")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} in {dir:?} failed");
    }

    fn git_available() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    /// Sets up a local "upstream" bare repo plus a shallow clone of it at
    /// `<canonical_root>/<name>`, entirely on-disk -- no network, no
    /// credentials, consistent with this project's policy of keeping real
    /// external-service calls out of automated tests.
    fn setup_git_skill(base: &Path, name: &str) -> (PathBuf, PathBuf) {
        let upstream = base.join("upstream.git");
        fs::create_dir_all(&upstream).unwrap();
        git(&upstream, &["init", "--quiet", "--bare"]);

        let seed = base.join("seed");
        fs::create_dir_all(&seed).unwrap();
        git(&seed, &["init", "--quiet"]);
        git(&seed, &["config", "user.email", "test@example.com"]);
        git(&seed, &["config", "user.name", "Test"]);
        fs::write(seed.join("SKILL.md"), "---\nname: x\ndescription: x\n---\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "--quiet", "-m", "initial"]);
        git(&seed, &["branch", "-M", "main"]);
        git(&seed, &["remote", "add", "origin", upstream.to_str().unwrap()]);
        git(&seed, &["push", "--quiet", "origin", "main"]);

        let canonical_root = base.join("canonical");
        fs::create_dir_all(&canonical_root).unwrap();
        let clone_status = Command::new("git")
            .arg("-c")
            .arg("safe.directory=*")
            .args(["clone", "--quiet", "--branch", "main"])
            .arg(&upstream)
            .arg(canonical_root.join(name))
            .status()
            .unwrap();
        assert!(clone_status.success());

        (upstream, canonical_root)
    }

    #[test]
    fn reports_up_to_date_immediately_after_cloning() {
        if !git_available() {
            eprintln!("skipping: git not available");
            return;
        }
        let base = TempDir::new("up-to-date");
        let (upstream, canonical_root) = setup_git_skill(&base.0, "my-skill");

        let index = SkillsIndex::open(base.0.join(".aam-skills-index.json"));
        let mut entry = SkillEntry::new_local("my-skill");
        entry.source = format!("{}@main", upstream.display());
        index.upsert(entry).unwrap();

        let statuses = check_updates_at(&canonical_root, &index).unwrap();
        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].up_to_date);
    }

    #[test]
    fn detects_and_applies_an_upstream_change() {
        if !git_available() {
            eprintln!("skipping: git not available");
            return;
        }
        let base = TempDir::new("has-update");
        let (upstream, canonical_root) = setup_git_skill(&base.0, "my-skill");

        // Push a new commit to "upstream" from a second working copy, so
        // the already-cloned canonical directory falls behind.
        //
        // Must pass `--branch main` explicitly, same as `setup_git_skill`'s
        // first clone: the bare repo's HEAD symbolic ref stays whatever
        // `init.defaultBranch` was at `git init --bare` time (unset by us),
        // which can differ from "main" depending on this git install's
        // config (confirmed: passed locally, failed on CI's windows-latest
        // runner with "remote HEAD refers to nonexistent ref" followed by
        // "src refspec main does not match any" on the later push --
        // `seed`'s `branch -M main` only renames *its own* branch, it never
        // touches the bare remote's HEAD pointer). Cloning without
        // `--branch` leaves this second clone with no local "main" to
        // commit onto at all.
        let second_clone = base.0.join("second-clone");
        let clone_status = Command::new("git")
            .arg("-c")
            .arg("safe.directory=*")
            .args(["clone", "--quiet", "--branch", "main"])
            .arg(&upstream)
            .arg(&second_clone)
            .status()
            .unwrap();
        assert!(clone_status.success());
        // This machine's `git` has no usable global user.name/email
        // (confirmed by running this test before adding these lines --
        // real "Author identity unknown" failure, not a hypothetical),
        // so every clone that commits needs its own local config, same
        // as `setup_git_skill` already does for `seed`.
        git(&second_clone, &["config", "user.email", "test@example.com"]);
        git(&second_clone, &["config", "user.name", "Test"]);
        fs::write(second_clone.join("SKILL.md"), "---\nname: x\ndescription: updated\n---\n").unwrap();
        git(&second_clone, &["add", "."]);
        git(&second_clone, &["commit", "--quiet", "-m", "update"]);
        git(&second_clone, &["push", "--quiet", "origin", "main"]);

        let index = SkillsIndex::open(base.0.join(".aam-skills-index.json"));
        let mut entry = SkillEntry::new_local("my-skill");
        entry.source = format!("{}@main", upstream.display());
        index.upsert(entry).unwrap();

        let statuses = check_updates_at(&canonical_root, &index).unwrap();
        assert!(!statuses[0].up_to_date);

        update_skill_at(&canonical_root, &index, "my-skill").unwrap();
        let content = fs::read_to_string(canonical_root.join("my-skill").join("SKILL.md")).unwrap();
        assert!(content.contains("updated"));

        let statuses_after = check_updates_at(&canonical_root, &index).unwrap();
        assert!(statuses_after[0].up_to_date);
    }

    #[test]
    fn check_updates_skips_local_sourced_entries() {
        let base = TempDir::new("skip-local");
        let index = SkillsIndex::open(base.0.join(".aam-skills-index.json"));
        index.upsert(SkillEntry::new_local("local-skill")).unwrap();

        let statuses = check_updates_at(&base.0.join("canonical"), &index).unwrap();
        assert!(statuses.is_empty());
    }

    #[test]
    fn update_skill_errors_for_local_sourced_entry() {
        let base = TempDir::new("update-local-errors");
        let index = SkillsIndex::open(base.0.join(".aam-skills-index.json"));
        index.upsert(SkillEntry::new_local("local-skill")).unwrap();

        let err = update_skill_at(&base.0.join("canonical"), &index, "local-skill").unwrap_err();
        assert!(matches!(err, UpdateError::NotGitSourced(_)));
    }
}
