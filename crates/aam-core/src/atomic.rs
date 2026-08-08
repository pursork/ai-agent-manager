use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic counter mixed into temp file names so that two `atomic_write`
/// calls for the same `path` within the same process never collide, even if
/// `std::process::id()` is constant for the process's whole lifetime.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Atomically replaces the contents of `path` with `contents`.
///
/// Writes to a hidden temporary file in the same directory as `path` (same
/// filesystem/volume, so the final rename is atomic on both Windows and
/// Unix), then renames it into place. If anything fails before the rename
/// completes, the temporary file is removed and `path` is left completely
/// untouched — there is no window where `path` contains partially-written
/// data.
///
/// This is the building block `TransactionalOp` implementations should use
/// for the "in-place file rewrite" backend described in
/// `docs/03-credential-account-module.md` §3.2/§3.5 (e.g. Codex's
/// `auth.json`), as opposed to the "N pre-materialized directories" backend
/// which never rewrites a live file at all.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic_write: path has no parent directory",
        )
    })?;

    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "atomic_write: path has no file name")
    })?;
    let file_name = file_name.to_string_lossy();

    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = dir.join(format!(".{file_name}.aam-tmp-{}-{unique}", std::process::id()));

    if let Err(e) = fs::write(&tmp_path, contents) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A directory under the OS temp dir, unique per test run, cleaned up on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "aam-core-test-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn writes_new_file() {
        let dir = TempDir::new("new-file");
        let target = dir.path().join("config.toml");

        atomic_write(&target, b"hello").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"hello");
        // no leftover temp files
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("aam-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
    }

    #[test]
    fn replaces_existing_file_without_partial_state() {
        let dir = TempDir::new("replace");
        let target = dir.path().join("auth.json");
        fs::write(&target, b"old-content").unwrap();

        atomic_write(&target, b"new-content").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new-content");
    }

    #[test]
    fn rejects_path_with_no_parent() {
        let result = atomic_write(Path::new("no-parent-marker-file"), b"x");
        // A bare relative filename has an empty parent (""), which we treat
        // as "no usable directory to stage the temp file in" rather than
        // silently writing into the process's current directory.
        assert!(result.is_err());
    }
}
