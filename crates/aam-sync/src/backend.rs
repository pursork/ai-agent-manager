//! Storage backend abstraction for the WebDAV directory layout
//! (`docs/04-webdav-sync-security.md` §4.5).
//!
//! The rest of this crate's business logic (age crypto, device manifest,
//! version-conflict handling) is written against [`SyncBackend`], not
//! against WebDAV directly, so it can be exercised in tests against
//! [`LocalDirBackend`] without a real server -- mirroring `aam-vault`'s
//! platform-conditional backend split (Windows DPAPI vs Unix `chmod 600`
//! behind one `SecretStore` shell).

use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Error from a [`SyncBackend`] operation. Deliberately opaque (a message
/// string) -- callers branch on `SyncBackend::get` returning `Ok(None)` for
/// "not found", not on error variants, so there is nothing callers need to
/// pattern-match here.
#[derive(Debug)]
pub struct BackendError(pub String);

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sync backend error: {}", self.0)
    }
}

impl Error for BackendError {}

/// Relative-path blob storage: `put`/`get`/`exists`/`delete` against paths
/// like `"devices.json.age"` or `"providers/cpa.blob.age"` (forward-slash
/// separated, matching §4.5's directory layout regardless of host OS).
pub trait SyncBackend {
    fn get(&self, path: &str) -> Result<Option<Vec<u8>>, BackendError>;
    fn put(&self, path: &str, bytes: &[u8]) -> Result<(), BackendError>;
    fn delete(&self, path: &str) -> Result<(), BackendError>;

    fn exists(&self, path: &str) -> Result<bool, BackendError> {
        Ok(self.get(path)?.is_some())
    }
}

/// Real backend: talks to a WebDAV server over HTTP (GET/PUT/MKCOL/DELETE +
/// Basic auth). Not covered by unit tests (needs a live server); exercised
/// by the user against their own WebDAV deployment.
pub struct WebDavBackend {
    base_url: String,
    username: String,
    password: String,
}

impl WebDavBackend {
    pub fn new(
        base_url: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        let base_url = base_url.into();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            username: username.into(),
            password: password.into(),
        }
    }

    fn url_for(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    fn authorization_header(&self) -> String {
        format!(
            "Basic {}",
            basic_auth_base64(&self.username, &self.password)
        )
    }

    /// WebDAV requires each collection (directory) in a path to exist
    /// before a `PUT` into it succeeds. Creates every parent collection
    /// via `MKCOL`, ignoring "already exists" failures -- MKCOL on an
    /// existing collection is expected to fail and that is not an error
    /// from this function's point of view.
    fn ensure_parent_dirs(&self, path: &str) -> Result<(), BackendError> {
        let Some(parent) = Path::new(path).parent() else {
            return Ok(());
        };
        let mut prefix = PathBuf::new();
        for component in parent.components() {
            prefix.push(component);
            let dir_path = prefix.to_string_lossy().replace('\\', "/");
            let url = self.url_for(&dir_path);
            let result = ureq::request("MKCOL", &url)
                .set("Authorization", &self.authorization_header())
                .timeout(Duration::from_secs(15))
                .call();
            // Any response at all (including 4xx "already exists") means
            // the server is reachable and the directory situation is
            // whatever it is; only a transport-level failure to even reach
            // the server is a real error here.
            if let Err(ureq::Error::Transport(t)) = result {
                return Err(BackendError(format!(
                    "MKCOL {dir_path} failed to reach server: {t}"
                )));
            }
        }
        Ok(())
    }
}

impl SyncBackend for WebDavBackend {
    fn get(&self, path: &str) -> Result<Option<Vec<u8>>, BackendError> {
        let url = self.url_for(path);
        match ureq::get(&url)
            .set("Authorization", &self.authorization_header())
            .timeout(Duration::from_secs(30))
            .call()
        {
            Ok(response) => {
                let mut body = Vec::new();
                response
                    .into_reader()
                    .read_to_end(&mut body)
                    .map_err(|e| BackendError(format!("reading GET {path} body: {e}")))?;
                Ok(Some(body))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(BackendError(format!("GET {path}: {e}"))),
        }
    }

    fn put(&self, path: &str, bytes: &[u8]) -> Result<(), BackendError> {
        self.ensure_parent_dirs(path)?;
        let url = self.url_for(path);
        ureq::put(&url)
            .set("Authorization", &self.authorization_header())
            .timeout(Duration::from_secs(30))
            .send_bytes(bytes)
            .map(|_| ())
            .map_err(|e| BackendError(format!("PUT {path}: {e}")))
    }

    fn delete(&self, path: &str) -> Result<(), BackendError> {
        let url = self.url_for(path);
        match ureq::delete(&url)
            .set("Authorization", &self.authorization_header())
            .timeout(Duration::from_secs(30))
            .call()
        {
            Ok(_) => Ok(()),
            // Deleting something that's already gone is not an error for
            // our callers (device revoke / blob cleanup are idempotent).
            Err(ureq::Error::Status(404, _)) => Ok(()),
            Err(e) => Err(BackendError(format!("DELETE {path}: {e}"))),
        }
    }
}

/// Minimal RFC 4648 standard base64 encoder (with padding), used only for
/// the HTTP `Authorization: Basic` header. Hand-rolled rather than pulling
/// in a dedicated `base64` crate as a direct dependency for one call site;
/// `age`'s dependency tree happens to include one but relying on a
/// transitive dependency's public API would be fragile.
fn basic_auth_base64(username: &str, password: &str) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input = format!("{username}:{password}");
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);

        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(b2 & 0x3f) as usize] as char
        } else {
            '='
        });
    }

    out
}

/// Test/local-development backend: maps the same relative paths onto files
/// under a root directory on disk. Lets the crate's business logic (crypto
/// round trips, device manifest edits, version-conflict handling) run in CI
/// without a real WebDAV server.
pub struct LocalDirBackend {
    root: PathBuf,
}

impl LocalDirBackend {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_for(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }
}

impl SyncBackend for LocalDirBackend {
    fn get(&self, path: &str) -> Result<Option<Vec<u8>>, BackendError> {
        let full = self.path_for(path);
        match fs::read(&full) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(BackendError(format!("reading {}: {e}", full.display()))),
        }
    }

    fn put(&self, path: &str, bytes: &[u8]) -> Result<(), BackendError> {
        let full = self.path_for(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| BackendError(format!("creating {}: {e}", parent.display())))?;
        }
        fs::write(&full, bytes).map_err(|e| BackendError(format!("writing {}: {e}", full.display())))
    }

    fn delete(&self, path: &str) -> Result<(), BackendError> {
        let full = self.path_for(path);
        match fs::remove_file(&full) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(BackendError(format!("deleting {}: {e}", full.display()))),
        }
    }
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
                "aam-sync-test-{label}-{}-{unique}",
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
    fn local_dir_backend_round_trips_and_reports_missing() {
        let dir = TempDir::new("roundtrip");
        let backend = LocalDirBackend::new(&dir.0);

        assert_eq!(backend.get("devices.json.age").unwrap(), None);
        assert!(!backend.exists("devices.json.age").unwrap());

        backend.put("devices.json.age", b"hello").unwrap();
        assert_eq!(
            backend.get("devices.json.age").unwrap(),
            Some(b"hello".to_vec())
        );
        assert!(backend.exists("devices.json.age").unwrap());

        backend.delete("devices.json.age").unwrap();
        assert_eq!(backend.get("devices.json.age").unwrap(), None);
    }

    #[test]
    fn local_dir_backend_creates_nested_directories() {
        let dir = TempDir::new("nested");
        let backend = LocalDirBackend::new(&dir.0);

        backend.put("providers/cpa.blob.age", b"secret").unwrap();
        assert_eq!(
            backend.get("providers/cpa.blob.age").unwrap(),
            Some(b"secret".to_vec())
        );
    }

    #[test]
    fn local_dir_backend_delete_of_missing_file_is_not_an_error() {
        let dir = TempDir::new("delete-missing");
        let backend = LocalDirBackend::new(&dir.0);
        backend.delete("nope.age").unwrap();
    }

    #[test]
    fn basic_auth_header_matches_known_vector() {
        // "Aladdin:open sesame" is the canonical RFC 7617 example.
        assert_eq!(
            basic_auth_base64("Aladdin", "open sesame"),
            "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }
}
