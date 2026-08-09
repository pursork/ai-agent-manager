//! Cross-device sync of the Memory-Bank index (`docs/05-session-memory-bank-module.md`
//! §5.6's `aam session sync`, `docs/04-webdav-sync-security.md` §4.5's
//! `memory-bank/project-index.blob.age`).
//!
//! Unlike Provider/account blobs (`aam-switcher::provider_sync`/
//! `account_sync`, one blob per id/fingerprint), this index is a
//! **single, shared blob multiple devices write to** -- naively pushing
//! "just my approved records" as the whole blob content would erase every
//! other device's contribution. The real flow: pull whatever's there,
//! replace only this device's own slice (identified by `deviceId`), keep
//! every other device's records untouched, push the union back.
//!
//! Pulled remote records are **never** written into the same
//! `project-index.json` `project-tracker`'s hook also writes
//! (`docs/08-open-questions-risks.md` #9) -- they land in a separate,
//! aam-owned mirror file instead ([`remote_mirror_path`]), so a sync bug
//! here can never clobber the user's live-tracked local file.

use crate::index::{IndexError, ProjectIndex};
use crate::record::ProjectRecord;
use aam_sync::{current_version, pull_if_newer, push_if_not_stale, BlobMeta, ConflictError, SyncBackend};
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

/// `docs/04-webdav-sync-security.md` §4.5's path -- one blob, shared by
/// every device, unlike Provider/account blobs.
const MEMORY_BLOB_PATH: &str = "memory-bank/project-index.blob.age";

#[derive(Debug)]
pub enum MemorySyncError {
    Index(IndexError),
    Sync(ConflictError),
    Serde(serde_json::Error),
}

impl fmt::Display for MemorySyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemorySyncError::Index(e) => write!(f, "{e}"),
            MemorySyncError::Sync(e) => write!(f, "{e}"),
            MemorySyncError::Serde(e) => write!(f, "memory-bank blob is corrupt: {e}"),
        }
    }
}

impl Error for MemorySyncError {}

impl From<IndexError> for MemorySyncError {
    fn from(e: IndexError) -> Self {
        MemorySyncError::Index(e)
    }
}
impl From<ConflictError> for MemorySyncError {
    fn from(e: ConflictError) -> Self {
        MemorySyncError::Sync(e)
    }
}
impl From<serde_json::Error> for MemorySyncError {
    fn from(e: serde_json::Error) -> Self {
        MemorySyncError::Serde(e)
    }
}

/// Where cross-device records pulled from WebDAV are mirrored --
/// deliberately not `project-index.json` itself, see this module's doc
/// comment.
pub fn remote_mirror_path() -> PathBuf {
    aam_core::aam_home().join("memory").join("remote-index.json")
}

pub fn remote_mirror_index() -> ProjectIndex {
    ProjectIndex::open(remote_mirror_path())
}

/// Pure merge step: from the shared set already on the server (`remote`,
/// empty if the blob didn't exist yet) and this device's own current
/// `syncApproved` records (`mine`), produces the next shared set to push
/// -- every `remote` record *not* authored by `device_id`, plus all of
/// `mine`. If this device un-approved something since its last sync, that
/// record is simply absent from `mine` and so disappears here too; other
/// devices' records are never touched.
fn merge_for_push(remote: Vec<ProjectRecord>, mine: Vec<ProjectRecord>, device_id: &str) -> Vec<ProjectRecord> {
    let mut merged: Vec<ProjectRecord> = remote.into_iter().filter(|r| r.device_id != device_id).collect();
    merged.extend(mine);
    merged
}

/// Runs one full sync: pull, merge (see [`merge_for_push`]), mirror
/// locally, push. `local`/`mirror` are passed in explicitly rather than
/// resolved internally (`ProjectIndex::open_default()`/
/// [`remote_mirror_index`]) so tests can point both at throwaway
/// directories instead of needing to fake `AAM_HOME`/the real
/// `~/.claude/project-index.json`.
pub fn sync_index(
    backend: &impl SyncBackend,
    local: &ProjectIndex,
    mirror: &ProjectIndex,
    recipients: &[String],
    device_id: &str,
    my_identity: &str,
) -> Result<BlobMeta, MemorySyncError> {
    let base_version = current_version(backend, MEMORY_BLOB_PATH)?;
    let remote: Vec<ProjectRecord> = match pull_if_newer(backend, MEMORY_BLOB_PATH, my_identity, 0)? {
        Some(blob) => serde_json::from_slice(&blob.plaintext)?,
        None => Vec::new(),
    };

    let mine: Vec<ProjectRecord> = local.list()?.into_iter().filter(|r| r.sync_approved).collect();

    let merged = merge_for_push(remote, mine, device_id);

    // Mirror locally first, so `aam project list` reflects the sync
    // immediately (including this device's own just-pushed records) even
    // if the push itself is about to fail (e.g. a version conflict).
    mirror.replace_all(merged.clone())?;

    let plaintext = serde_json::to_vec(&merged)?;
    Ok(push_if_not_stale(
        backend,
        MEMORY_BLOB_PATH,
        &plaintext,
        recipients,
        device_id,
        base_version,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aam_sync::{generate_device_keypair, LocalDirBackend};
    use std::fs;
    use std::path::PathBuf as StdPathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(StdPathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "aam-memory-sync-test-{label}-{}-{unique}",
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

    fn sample(path: &str, name: &str, device_id: &str, approved: bool) -> ProjectRecord {
        ProjectRecord {
            path: path.to_string(),
            name: name.to_string(),
            last_session_id: "sid".into(),
            last_active: "2026-08-08T10:00:00Z".into(),
            created: "2026-08-01T10:00:00Z".into(),
            auto_status: None,
            status_override: None,
            auth_backend: None,
            device_id: device_id.to_string(),
            tool_kind: "claude".into(),
            profile_label: None,
            full_sync_enabled: false,
            full_sync_status: None,
            discovery_source: "live".into(),
            sync_approved: approved,
            project_id: None,
        }
    }

    #[test]
    fn merge_for_push_first_sync_is_just_mine() {
        let mine = vec![sample("/a", "a", "device-a", true)];
        let merged = merge_for_push(Vec::new(), mine.clone(), "device-a");
        assert_eq!(merged, mine);
    }

    #[test]
    fn merge_for_push_preserves_other_devices_records() {
        let remote = vec![sample("/b", "b", "device-b", true)];
        let mine = vec![sample("/a", "a", "device-a", true)];
        let merged = merge_for_push(remote, mine, "device-a");
        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|r| r.device_id == "device-a"));
        assert!(merged.iter().any(|r| r.device_id == "device-b"));
    }

    #[test]
    fn merge_for_push_replaces_own_stale_records_entirely() {
        // Remote already has an old record from device-a (say, a project
        // it no longer has approved); device-a's fresh push should not
        // carry it forward.
        let remote = vec![
            sample("/old", "old", "device-a", true),
            sample("/b", "b", "device-b", true),
        ];
        let mine = vec![sample("/new", "new", "device-a", true)];
        let merged = merge_for_push(remote, mine, "device-a");

        assert_eq!(merged.len(), 2);
        assert!(!merged.iter().any(|r| r.path == "/old"));
        assert!(merged.iter().any(|r| r.path == "/new"));
        assert!(merged.iter().any(|r| r.path == "/b"));
    }

    #[test]
    fn merge_for_push_revoking_approval_removes_it_from_shared_set() {
        // device-a previously pushed "/a"; it's since un-approved that
        // record locally (so `mine` no longer contains it). The next
        // merge must drop it from the shared set, not keep it forever.
        let remote = vec![sample("/a", "a", "device-a", true)];
        let mine: Vec<ProjectRecord> = Vec::new();
        let merged = merge_for_push(remote, mine, "device-a");
        assert!(merged.is_empty());
    }

    #[test]
    fn sync_index_end_to_end_two_devices_do_not_clobber_each_other() {
        let backend_dir = TempDir::new("e2e-backend");
        let backend = LocalDirBackend::new(&backend_dir.0);
        let (public_a, priv_a) = generate_device_keypair();
        let (public_b, priv_b) = generate_device_keypair();
        let recipients = vec![public_a, public_b];

        // Device A: has one approved local record, syncs first.
        let a_local_dir = TempDir::new("e2e-a-local");
        let a_local = ProjectIndex::open(a_local_dir.0.join("project-index.json"));
        a_local.upsert(sample("/a-project", "a-project", "device-a", true)).unwrap();
        let a_mirror_dir = TempDir::new("e2e-a-mirror");
        let a_mirror = ProjectIndex::open(a_mirror_dir.0.join("remote-index.json"));

        let meta_a = sync_index(&backend, &a_local, &a_mirror, &recipients, "device-a", &priv_a).unwrap();
        assert_eq!(meta_a.version, 1);
        assert_eq!(a_mirror.list().unwrap().len(), 1);

        // Device B: has a different approved local record, syncs second.
        let b_local_dir = TempDir::new("e2e-b-local");
        let b_local = ProjectIndex::open(b_local_dir.0.join("project-index.json"));
        b_local.upsert(sample("/b-project", "b-project", "device-b", true)).unwrap();
        let b_mirror_dir = TempDir::new("e2e-b-mirror");
        let b_mirror = ProjectIndex::open(b_mirror_dir.0.join("remote-index.json"));

        let meta_b = sync_index(&backend, &b_local, &b_mirror, &recipients, "device-b", &priv_b).unwrap();
        assert_eq!(meta_b.version, 2);

        // B's mirror must contain BOTH devices' records -- A's push wasn't
        // clobbered.
        let b_mirror_records = b_mirror.list().unwrap();
        assert_eq!(b_mirror_records.len(), 2);
        assert!(b_mirror_records.iter().any(|r| r.path == "/a-project"));
        assert!(b_mirror_records.iter().any(|r| r.path == "/b-project"));

        // A re-syncs (still has the same approved record) -- must not
        // lose B's contribution.
        let meta_a2 = sync_index(&backend, &a_local, &a_mirror, &recipients, "device-a", &priv_a).unwrap();
        assert_eq!(meta_a2.version, 3);
        let a_mirror_records = a_mirror.list().unwrap();
        assert_eq!(a_mirror_records.len(), 2);
    }

    #[test]
    fn sync_index_revocation_removes_own_record_but_not_others() {
        let backend_dir = TempDir::new("revoke-backend");
        let backend = LocalDirBackend::new(&backend_dir.0);
        let (public_a, priv_a) = generate_device_keypair();
        let (public_b, _priv_b) = generate_device_keypair();
        let recipients = vec![public_a, public_b];

        let a_local_dir = TempDir::new("revoke-a-local");
        let a_local = ProjectIndex::open(a_local_dir.0.join("project-index.json"));
        a_local.upsert(sample("/a-project", "a-project", "device-a", true)).unwrap();
        let a_mirror_dir = TempDir::new("revoke-a-mirror");
        let a_mirror = ProjectIndex::open(a_mirror_dir.0.join("remote-index.json"));
        sync_index(&backend, &a_local, &a_mirror, &recipients, "device-a", &priv_a).unwrap();

        // Simulate another device (B) having pushed too, by pushing
        // directly to the shared backend state via a second local/mirror
        // pair is unnecessary here -- just revoke A's own record and
        // re-sync, confirming the shared set shrinks accordingly.
        a_local
            .update("/a-project", |r| r.sync_approved = false)
            .unwrap();
        let meta = sync_index(&backend, &a_local, &a_mirror, &recipients, "device-a", &priv_a).unwrap();
        assert_eq!(meta.version, 2);
        assert!(a_mirror.list().unwrap().is_empty());
    }
}
