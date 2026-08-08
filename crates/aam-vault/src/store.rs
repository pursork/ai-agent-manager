use aam_core::atomic_write;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

#[cfg(windows)]
use crate::windows_dpapi;
#[cfg(unix)]
use crate::unix_plain;

/// Error type for all [`SecretStore`] operations.
#[derive(Debug)]
pub enum VaultError {
    Io(io::Error),
    /// Backend-specific failure (e.g. the Windows DPAPI helper process
    /// failed, or a decrypted value wasn't valid UTF-8).
    Backend(String),
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VaultError::Io(e) => write!(f, "vault I/O error: {e}"),
            VaultError::Backend(msg) => write!(f, "vault backend error: {msg}"),
        }
    }
}

impl Error for VaultError {}

impl From<io::Error> for VaultError {
    fn from(e: io::Error) -> Self {
        VaultError::Io(e)
    }
}

/// A local, OS-appropriate encrypted-at-rest secret store.
///
/// One `SecretStore` owns one directory (`root`) and one `entropy` label.
/// `entropy` is mixed into the Windows DPAPI call (see `windows_dpapi`) as
/// an additional input, following the same convention `codex-skill` uses
/// (a fixed per-purpose string, not a real secret rotation mechanism) —
/// callers that want isolation between unrelated secret categories should
/// use different `entropy` strings, not just different `root` directories.
pub struct SecretStore {
    root: PathBuf,
    entropy: &'static str,
}

impl SecretStore {
    /// Opens (creating if necessary) a secret store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>, entropy: &'static str) -> io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root, entropy })
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.root.join(format!("{}.secret", sanitize_key(key)))
    }

    /// Encrypts (Windows) or stores (Unix, with restricted permissions)
    /// `plaintext` under `key`, overwriting any existing value atomically.
    pub fn save(&self, key: &str, plaintext: &str) -> Result<(), VaultError> {
        let path = self.path_for(key);

        #[cfg(windows)]
        let bytes = windows_dpapi::protect(&self.root, plaintext, self.entropy)?;
        #[cfg(unix)]
        let bytes = plaintext.as_bytes().to_vec();

        atomic_write(&path, &bytes)?;

        #[cfg(unix)]
        unix_plain::restrict_permissions(&path)?;

        Ok(())
    }

    /// Loads and decrypts the value stored under `key`, or `None` if it
    /// was never saved (or has since been deleted).
    pub fn load(&self, key: &str) -> Result<Option<String>, VaultError> {
        let path = self.path_for(key);
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = fs::read(&path)?;

        #[cfg(windows)]
        let text = windows_dpapi::unprotect(&self.root, &bytes, self.entropy)?;
        #[cfg(unix)]
        let text = String::from_utf8(bytes)
            .map_err(|e| VaultError::Backend(format!("stored secret is not valid UTF-8: {e}")))?;

        Ok(Some(text))
    }

    /// Removes the value stored under `key`, if any. Not an error if it
    /// was already absent.
    pub fn delete(&self, key: &str) -> Result<(), VaultError> {
        let path = self.path_for(key);
        if path.is_file() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

/// Keys are used as filename stems, so anything outside a conservative
/// ASCII allowlist is replaced with `_` rather than trusted verbatim.
fn sanitize_key(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
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
                "aam-vault-test-{label}-{}-{unique}",
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

    #[test]
    fn round_trips_a_secret() {
        let dir = TempDir::new("roundtrip");
        let store = SecretStore::new(&dir.0, "aam-vault-test-v1").unwrap();

        store.save("deepseek-api-key", "sk-test-1234567890").unwrap();
        let loaded = store.load("deepseek-api-key").unwrap();

        assert_eq!(loaded.as_deref(), Some("sk-test-1234567890"));
    }

    #[test]
    fn missing_key_returns_none() {
        let dir = TempDir::new("missing");
        let store = SecretStore::new(&dir.0, "aam-vault-test-v1").unwrap();

        assert_eq!(store.load("never-saved").unwrap(), None);
    }

    #[test]
    fn delete_removes_the_value() {
        let dir = TempDir::new("delete");
        let store = SecretStore::new(&dir.0, "aam-vault-test-v1").unwrap();

        store.save("cpa-api-key", "secret-value").unwrap();
        assert!(store.load("cpa-api-key").unwrap().is_some());

        store.delete("cpa-api-key").unwrap();
        assert_eq!(store.load("cpa-api-key").unwrap(), None);

        // deleting an already-absent key is not an error
        store.delete("cpa-api-key").unwrap();
    }

    #[test]
    fn overwrite_replaces_the_value() {
        let dir = TempDir::new("overwrite");
        let store = SecretStore::new(&dir.0, "aam-vault-test-v1").unwrap();

        store.save("k", "first").unwrap();
        store.save("k", "second").unwrap();

        assert_eq!(store.load("k").unwrap().as_deref(), Some("second"));
    }

    #[cfg(windows)]
    #[test]
    fn ciphertext_is_not_the_plaintext_on_windows() {
        let dir = TempDir::new("ciphertext");
        let store = SecretStore::new(&dir.0, "aam-vault-test-v1").unwrap();
        store.save("k", "not-encrypted-would-be-bad").unwrap();

        let on_disk = fs::read(dir.0.join("k.secret")).unwrap();
        let on_disk_text = String::from_utf8_lossy(&on_disk);
        assert!(
            !on_disk_text.contains("not-encrypted-would-be-bad"),
            "secret must not appear in plaintext on disk on Windows"
        );
    }
}
