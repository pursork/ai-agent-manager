//! Versioned, multi-recipient-encrypted blobs (`docs/04-webdav-sync-security.md`
//! §4.5/§4.6): every blob (`providers/<id>.blob.age`,
//! `memory-bank/project-index.blob.age`, ...) is stored as two objects at
//! the same `SyncBackend` path --
//!
//! - `<path>` -- the `age` ciphertext.
//! - `<path>.meta.json` -- **unencrypted** metadata (`version` /
//!   `updated_at` / `updated_by_device`), used for §4.6's conflict
//!   detection without needing to decrypt anything. The metadata never
//!   contains secrets, so plaintext storage doesn't weaken the zero-
//!   knowledge goal (§4.5's own text says as much).
//!
//! §4.6's conflict rule, implemented here exactly as specified: **not** a
//! real diff/merge -- whole-blob overwrite gated on a monotonically
//! increasing `version` number. A push is rejected (not silently
//! overwritten) if the remote version has moved past what the caller last
//! knew about.

use crate::age_crypto::{decrypt_multi_recipient, encrypt_multi_recipient, CryptoError};
use crate::backend::{BackendError, SyncBackend};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlobMeta {
    pub version: u64,
    /// RFC 3339 timestamp, e.g. `2026-08-08T12:00:00Z`.
    pub updated_at: String,
    pub updated_by_device: String,
}

/// A blob successfully pulled: its metadata plus the decrypted payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedBlob {
    pub meta: BlobMeta,
    pub plaintext: Vec<u8>,
}

#[derive(Debug)]
pub enum ConflictError {
    /// §4.6: "推送时如果远端 version 已经比本地准备推送的更高 → 中止推送，
    /// 先拉取合并" -- the caller must `pull_if_newer` first and retry.
    RemoteIsNewer {
        remote_version: u64,
        local_base_version: u64,
    },
    Backend(BackendError),
    Crypto(CryptoError),
    Serde(serde_json::Error),
}

impl fmt::Display for ConflictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConflictError::RemoteIsNewer { remote_version, local_base_version } => write!(
                f,
                "another device already pushed version {remote_version} (you have version \
                 {local_base_version}) -- pull the latest version before pushing again"
            ),
            ConflictError::Backend(e) => write!(f, "{e}"),
            ConflictError::Crypto(e) => write!(f, "{e}"),
            ConflictError::Serde(e) => write!(f, "blob metadata JSON error: {e}"),
        }
    }
}

impl Error for ConflictError {}

impl From<BackendError> for ConflictError {
    fn from(e: BackendError) -> Self {
        ConflictError::Backend(e)
    }
}

impl From<CryptoError> for ConflictError {
    fn from(e: CryptoError) -> Self {
        ConflictError::Crypto(e)
    }
}

impl From<serde_json::Error> for ConflictError {
    fn from(e: serde_json::Error) -> Self {
        ConflictError::Serde(e)
    }
}

fn meta_path(path: &str) -> String {
    format!("{path}.meta.json")
}

fn read_meta(backend: &impl SyncBackend, path: &str) -> Result<Option<BlobMeta>, ConflictError> {
    match backend.get(&meta_path(path))? {
        Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        None => Ok(None),
    }
}

/// Reads just a blob's current version -- no decryption needed, since
/// metadata is plaintext. `0` if the blob doesn't exist yet. Callers use
/// this to compute `local_base_version` for [`push_if_not_stale`] without
/// needing to decrypt (or even be a recipient of) the existing blob first.
pub fn current_version(backend: &impl SyncBackend, path: &str) -> Result<u64, ConflictError> {
    Ok(read_meta(backend, path)?.map(|m| m.version).unwrap_or(0))
}

/// Encrypts `plaintext` to `recipients` and writes it (+ metadata) to
/// `path`, **unless** another device has pushed a newer version since the
/// caller last knew about `local_base_version` (pass `0` when creating a
/// brand new blob). Returns the new metadata on success.
pub fn push_if_not_stale(
    backend: &impl SyncBackend,
    path: &str,
    plaintext: &[u8],
    recipients: &[String],
    updated_by_device: &str,
    local_base_version: u64,
) -> Result<BlobMeta, ConflictError> {
    let remote_version = read_meta(backend, path)?.map(|m| m.version).unwrap_or(0);
    if remote_version > local_base_version {
        return Err(ConflictError::RemoteIsNewer {
            remote_version,
            local_base_version,
        });
    }

    let ciphertext = encrypt_multi_recipient(plaintext, recipients)?;
    backend.put(path, &ciphertext)?;

    let meta = BlobMeta {
        version: remote_version + 1,
        updated_at: crate::util::now_rfc3339(),
        updated_by_device: updated_by_device.to_string(),
    };
    backend.put(&meta_path(path), &serde_json::to_vec_pretty(&meta)?)?;
    Ok(meta)
}

/// Pulls and decrypts `path` with `identity` **only if** the remote version
/// is newer than `local_known_version` -- returns `None` if there is
/// nothing new (including "the blob doesn't exist yet", version 0).
pub fn pull_if_newer(
    backend: &impl SyncBackend,
    path: &str,
    identity: &str,
    local_known_version: u64,
) -> Result<Option<VersionedBlob>, ConflictError> {
    let Some(meta) = read_meta(backend, path)? else {
        return Ok(None);
    };
    if meta.version <= local_known_version {
        return Ok(None);
    }

    let ciphertext = backend.get(path)?.ok_or_else(|| {
        ConflictError::Backend(BackendError(format!(
            "{path}.meta.json exists but {path} itself is missing (corrupt or partial write)"
        )))
    })?;
    let plaintext = decrypt_multi_recipient(&ciphertext, identity)?;
    Ok(Some(VersionedBlob { meta, plaintext }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::age_crypto::generate_device_keypair;
    use crate::backend::LocalDirBackend;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "aam-sync-blob-test-{label}-{}-{unique}",
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
    fn push_then_pull_round_trip() {
        let dir = TempDir::new("roundtrip");
        let backend = LocalDirBackend::new(&dir.0);
        let (public, private) = generate_device_keypair();

        let meta = push_if_not_stale(
            &backend,
            "providers/cpa.blob.age",
            b"provider config v1",
            &[public],
            "device-a",
            0,
        )
        .unwrap();
        assert_eq!(meta.version, 1);

        let pulled = pull_if_newer(&backend, "providers/cpa.blob.age", &private, 0)
            .unwrap()
            .expect("should have found a newer blob");
        assert_eq!(pulled.plaintext, b"provider config v1");
        assert_eq!(pulled.meta.version, 1);
    }

    #[test]
    fn pull_returns_none_when_nothing_new() {
        let dir = TempDir::new("no-new");
        let backend = LocalDirBackend::new(&dir.0);
        let (public, private) = generate_device_keypair();

        let meta = push_if_not_stale(&backend, "p", b"v1", &[public], "device-a", 0).unwrap();

        // Caller already knows about this exact version -- nothing to pull.
        let pulled = pull_if_newer(&backend, "p", &private, meta.version).unwrap();
        assert!(pulled.is_none());
    }

    #[test]
    fn pull_of_nonexistent_blob_returns_none() {
        let dir = TempDir::new("missing");
        let backend = LocalDirBackend::new(&dir.0);
        let (_public, private) = generate_device_keypair();

        let pulled = pull_if_newer(&backend, "nope", &private, 0).unwrap();
        assert!(pulled.is_none());
    }

    /// The core of §4.6: pushing on top of a stale base version must be
    /// rejected, not silently overwrite another device's newer write.
    #[test]
    fn push_on_stale_base_version_is_rejected() {
        let dir = TempDir::new("conflict");
        let backend = LocalDirBackend::new(&dir.0);
        let (public_a, _priv_a) = generate_device_keypair();
        let (public_b, _priv_b) = generate_device_keypair();

        // Device A pushes version 1.
        push_if_not_stale(&backend, "p", b"from-a", std::slice::from_ref(&public_a), "device-a", 0)
            .unwrap();

        // Device B, still thinking the blob doesn't exist (base version 0),
        // tries to push -- must be rejected because A already moved it to 1.
        let err = push_if_not_stale(&backend, "p", b"from-b", &[public_b], "device-b", 0)
            .unwrap_err();
        assert!(matches!(
            err,
            ConflictError::RemoteIsNewer {
                remote_version: 1,
                local_base_version: 0
            }
        ));

        // The rejected push must not have touched the stored blob.
        let (_pub_a2, priv_a) = (public_a, _priv_a);
        let pulled = pull_if_newer(&backend, "p", &priv_a, 0).unwrap().unwrap();
        assert_eq!(pulled.plaintext, b"from-a");
    }

    #[test]
    fn push_on_correct_base_version_succeeds_and_increments() {
        let dir = TempDir::new("increment");
        let backend = LocalDirBackend::new(&dir.0);
        let (public, private) = generate_device_keypair();

        let meta1 =
            push_if_not_stale(&backend, "p", b"v1", std::slice::from_ref(&public), "device-a", 0)
                .unwrap();
        assert_eq!(meta1.version, 1);

        let meta2 =
            push_if_not_stale(&backend, "p", b"v2", &[public], "device-a", meta1.version).unwrap();
        assert_eq!(meta2.version, 2);

        let pulled = pull_if_newer(&backend, "p", &private, 0).unwrap().unwrap();
        assert_eq!(pulled.plaintext, b"v2");
        assert_eq!(pulled.meta.version, 2);
    }
}
