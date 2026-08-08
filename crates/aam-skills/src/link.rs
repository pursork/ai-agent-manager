//! Cross-platform "make this directory path resolve to that directory
//! path" primitive: a Windows Junction (`mklink /J`) or a Unix directory
//! symlink, wrapped as an `aam_core::TransactionalOp` per
//! `docs/02-architecture.md` §2.6.
//!
//! Windows uses a Junction rather than a real symlink deliberately
//! (`docs/09-skills-management.md` §9.4): symlinks need elevation or
//! Developer Mode, Junctions don't, and this project already paid the
//! "needs elevation" tax once during Phase 0 (the Visual Studio Build
//! Tools install) and isn't eager to require it again for something this
//! routine.

use aam_core::TransactionalOp;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum LinkError {
    Io(io::Error),
    /// `link_path` already exists as something other than a link that
    /// resolves to `target_path` (a real directory with content, or a
    /// link elsewhere) -- refusing to silently delete/replace it, per
    /// `docs/09-skills-management.md` §9.3's "先给出提示，不能不问用户就
    /// 删了替换" principle.
    Occupied(PathBuf),
    /// The platform-specific link command itself failed (e.g. `mklink /J`
    /// exited non-zero).
    Backend(String),
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::Io(e) => write!(f, "I/O error: {e}"),
            LinkError::Occupied(path) => write!(
                f,
                "{} already exists and is not a link to the expected target; refusing to touch it",
                path.display()
            ),
            LinkError::Backend(msg) => write!(f, "{msg}"),
        }
    }
}

impl Error for LinkError {}

impl From<io::Error> for LinkError {
    fn from(e: io::Error) -> Self {
        LinkError::Io(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkState {
    Absent,
    LinksToTarget,
    OccupiedByOther,
}

fn inspect(link_path: &Path, target_path: &Path) -> io::Result<LinkState> {
    let link_meta = match fs::symlink_metadata(link_path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(LinkState::Absent),
        Err(e) => return Err(e),
    };

    if !is_link_like(&link_meta) {
        return Ok(LinkState::OccupiedByOther);
    }

    let link_resolved = fs::canonicalize(link_path).ok();
    let target_resolved = fs::canonicalize(target_path).ok();
    match (link_resolved, target_resolved) {
        (Some(a), Some(b)) if a == b => Ok(LinkState::LinksToTarget),
        _ => Ok(LinkState::OccupiedByOther),
    }
}

#[cfg(windows)]
fn is_link_like(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(unix)]
fn is_link_like(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

#[cfg(windows)]
fn windows_backslash_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\")
}

#[cfg(windows)]
fn create_dir_link(link_path: &Path, target_path: &Path) -> Result<(), LinkError> {
    // mklink /J requires the target to already exist.
    fs::create_dir_all(target_path)?;
    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // cmd.exe's built-in `mklink` mis-tokenizes forward slashes in an
    // unquoted path (it reads `/` as a switch prefix, e.g. `C:/Users/...`
    // gets misread as drive `C:` followed by a bogus `/Users` switch,
    // failing with "invalid drive"). PathBuf happily stores forward
    // slashes verbatim (e.g. when built from an `AAM_HOME` env var that
    // used them), so normalize to backslashes right before this
    // cmd.exe-specific call rather than assuming callers never produce
    // mixed-separator paths.
    let output = std::process::Command::new("cmd.exe")
        .args(["/d", "/s", "/c", "mklink", "/J"])
        .arg(windows_backslash_path(link_path))
        .arg(windows_backslash_path(target_path))
        .output()
        .map_err(|e| LinkError::Backend(format!("failed to spawn cmd.exe for mklink: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(LinkError::Backend(format!(
            "mklink /J {} {} failed ({}): {}{}",
            link_path.display(),
            target_path.display(),
            output.status,
            stderr.trim(),
            stdout.trim()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn create_dir_link(link_path: &Path, target_path: &Path) -> Result<(), LinkError> {
    fs::create_dir_all(target_path)?;
    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent)?;
    }
    std::os::unix::fs::symlink(target_path, link_path)?;
    Ok(())
}

/// Removes the link/Junction at `link_path` itself, without touching
/// whatever it points to -- `remove_dir` on a reparse point unlinks the
/// reparse point only, both on Windows (Junctions/symlinks) and Unix
/// (directory symlinks).
fn remove_dir_link(link_path: &Path) -> io::Result<()> {
    fs::remove_dir(link_path)
}

/// Reports whether `link_path` currently resolves to `target_path` via a
/// link (Junction/symlink) -- read-only, used by `list`/`status` to report
/// existing state without attempting to change anything.
pub fn resolves_to(link_path: &Path, target_path: &Path) -> bool {
    matches!(inspect(link_path, target_path), Ok(LinkState::LinksToTarget))
}

/// Provisions `link_path` to resolve to `target_path` (Junction on
/// Windows, directory symlink on Unix), as an `aam_core::TransactionalOp`.
///
/// - Already correctly linked → `apply` is a no-op.
/// - Absent → creates `target_path` if needed, then the link.
/// - Occupied by something else → `apply` fails with [`LinkError::Occupied`]
///   rather than touching it.
pub struct ProvisionDirLink {
    link_path: PathBuf,
    target_path: PathBuf,
    created_by_us: bool,
}

impl ProvisionDirLink {
    pub fn new(link_path: impl Into<PathBuf>, target_path: impl Into<PathBuf>) -> Self {
        Self {
            link_path: link_path.into(),
            target_path: target_path.into(),
            created_by_us: false,
        }
    }
}

impl TransactionalOp for ProvisionDirLink {
    type Snapshot = ();
    type Error = LinkError;

    fn snapshot(&self) -> Result<(), LinkError> {
        // Nothing to capture: if `link_path` already exists as something
        // other than the target link, `apply` refuses rather than
        // overwriting it, so there's no prior state that ever needs
        // restoring on rollback -- only "undo what *this* apply created".
        Ok(())
    }

    fn apply(&mut self) -> Result<(), LinkError> {
        match inspect(&self.link_path, &self.target_path)? {
            LinkState::LinksToTarget => {
                self.created_by_us = false;
                Ok(())
            }
            LinkState::Absent => {
                create_dir_link(&self.link_path, &self.target_path)?;
                self.created_by_us = true;
                Ok(())
            }
            LinkState::OccupiedByOther => Err(LinkError::Occupied(self.link_path.clone())),
        }
    }

    fn verify(&self) -> Result<(), LinkError> {
        match inspect(&self.link_path, &self.target_path)? {
            LinkState::LinksToTarget => Ok(()),
            _ => Err(LinkError::Backend(format!(
                "{} does not resolve to {} after provisioning",
                self.link_path.display(),
                self.target_path.display()
            ))),
        }
    }

    fn rollback(&mut self, _snapshot: ()) -> Result<(), LinkError> {
        if self.created_by_us {
            remove_dir_link(&self.link_path)?;
            self.created_by_us = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aam_core::execute;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "aam-skills-test-{label}-{}-{unique}",
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

    #[cfg(windows)]
    #[test]
    fn provisions_a_link_even_when_the_path_has_forward_slashes() {
        // Regression test: an AAM_HOME (or other) env var set with
        // forward slashes (e.g. from a POSIX-style shell) used to make
        // cmd.exe's `mklink` fail with "invalid drive" because it
        // misreads unquoted `/` as a switch prefix.
        let base = TempDir::new("forward-slashes");
        let target = base.0.join("canonical");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("marker.txt"), b"hi").unwrap();

        let forward_slash_base = base.0.to_string_lossy().replace('\\', "/");
        let link = PathBuf::from(format!("{forward_slash_base}/profile/skills"));

        let mut op = ProvisionDirLink::new(&link, &target);
        execute(&mut op).expect("provisioning should succeed even with forward slashes in the path");
        assert!(link.join("marker.txt").is_file());
    }

    #[test]
    fn provisions_a_new_link_and_it_resolves_to_target_content() {
        let base = TempDir::new("new-link");
        let target = base.0.join("canonical");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("marker.txt"), b"hello").unwrap();

        let link = base.0.join("profile").join("skills");
        let mut op = ProvisionDirLink::new(&link, &target);
        execute(&mut op).expect("provisioning should succeed");

        assert!(link.join("marker.txt").is_file(), "link should expose target's content");
        assert_eq!(fs::read_to_string(link.join("marker.txt")).unwrap(), "hello");
    }

    #[test]
    fn re_running_on_an_already_correct_link_is_a_no_op() {
        let base = TempDir::new("idempotent");
        let target = base.0.join("canonical");
        fs::create_dir_all(&target).unwrap();
        let link = base.0.join("skills");

        let mut op1 = ProvisionDirLink::new(&link, &target);
        execute(&mut op1).unwrap();

        let mut op2 = ProvisionDirLink::new(&link, &target);
        execute(&mut op2).expect("second provisioning of the same link should succeed as a no-op");
    }

    #[test]
    fn refuses_to_touch_a_real_directory_with_content() {
        let base = TempDir::new("occupied");
        let target = base.0.join("canonical");
        fs::create_dir_all(&target).unwrap();

        let link = base.0.join("skills");
        fs::create_dir_all(&link).unwrap();
        fs::write(link.join("do-not-delete-me.txt"), b"user content").unwrap();

        let mut op = ProvisionDirLink::new(&link, &target);
        let result = execute(&mut op);
        assert!(result.is_err(), "must refuse to replace a real directory");
        assert!(
            link.join("do-not-delete-me.txt").is_file(),
            "user content must survive the refused operation"
        );
    }

    #[test]
    fn rollback_removes_the_link_but_not_the_target_content() {
        let base = TempDir::new("rollback");
        let target = base.0.join("canonical");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("keep-me.txt"), b"target content").unwrap();

        let link = base.0.join("skills");
        let mut op = ProvisionDirLink::new(&link, &target);
        op.apply().expect("apply should succeed");
        assert!(link.join("keep-me.txt").is_file());

        op.rollback(()).expect("rollback should succeed");
        assert!(!link.exists(), "link itself should be gone after rollback");
        assert!(
            target.join("keep-me.txt").is_file(),
            "target content must be untouched by rollback"
        );
    }
}
