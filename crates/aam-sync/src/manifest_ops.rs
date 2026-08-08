//! High-level device-manifest flows (`docs/04-webdav-sync-security.md`
//! §4.3/§4.4): initializing a brand new vault, joining an existing one, and
//! revoking a device. Ties together [`crate::backend`], [`crate::age_crypto`],
//! and [`crate::device`], plus this machine's own local age identity
//! (persisted via `aam-vault::SecretStore`, never uploaded to the backend).
//!
//! **Deliberately not implemented here**: §4.3 step 6, "re-encrypt every
//! existing blob so the newly-joined device can read them." That requires
//! knowing the full set of blob paths, which this crate -- by design (see
//! `lib.rs`) -- has no way to enumerate, since it has no knowledge of
//! domain-specific paths like `providers/<id>.blob.age`. Re-encryption is
//! composable by callers using [`crate::pull_if_newer`] (with your own
//! identity) + [`crate::push_if_not_stale`] (with the updated recipient
//! list) for each blob path *they* know about -- see `aam-switcher`'s
//! provider sync module for the concrete instance of this pattern.

use crate::age_crypto::{decrypt_with_passphrase, encrypt_with_passphrase, generate_device_keypair, CryptoError};
use crate::backend::{BackendError, SyncBackend};
use crate::device::{join_device, revoke_device, DeviceEntry, DeviceError, DeviceManifest};
use crate::util::now_rfc3339;
use aam_vault::{SecretStore, VaultError};
use std::error::Error;
use std::fmt;
use std::path::Path;

/// Well-known path for the device manifest (`§4.5`).
pub const DEVICES_MANIFEST_PATH: &str = "devices.json.age";

const DEVICE_IDENTITY_ENTROPY: &str = "ai-agent-manager-aam-sync-device-identity-v1";
const DEVICE_ID_KEY: &str = "device_id";
const PUBLIC_KEY_KEY: &str = "public_key";
const PRIVATE_KEY_KEY: &str = "private_key";

#[derive(Debug)]
pub enum ManifestOpError {
    Backend(BackendError),
    Crypto(CryptoError),
    Serde(serde_json::Error),
    Device(DeviceError),
    Vault(VaultError),
    /// `init_vault` was called but a manifest already exists at
    /// [`DEVICES_MANIFEST_PATH`] -- use `join_device_to_vault` instead.
    AlreadyInitialized,
    /// `join_device_to_vault`/`list_devices`/`revoke_device_in_vault` was
    /// called but no manifest exists yet -- use `init_vault` first.
    NotInitialized,
}

impl fmt::Display for ManifestOpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestOpError::Backend(e) => write!(f, "{e}"),
            ManifestOpError::Crypto(e) => write!(f, "{e}"),
            ManifestOpError::Serde(e) => write!(f, "devices.json is corrupt: {e}"),
            ManifestOpError::Device(e) => write!(f, "{e}"),
            ManifestOpError::Vault(e) => write!(f, "local device identity storage error: {e}"),
            ManifestOpError::AlreadyInitialized => write!(
                f,
                "this WebDAV location already has a vault set up (devices.json.age exists) -- \
                 use `aam device join` instead of `aam sync init`"
            ),
            ManifestOpError::NotInitialized => write!(
                f,
                "no vault found at this WebDAV location yet -- run `aam sync init` first"
            ),
        }
    }
}

impl Error for ManifestOpError {}

impl From<BackendError> for ManifestOpError {
    fn from(e: BackendError) -> Self {
        ManifestOpError::Backend(e)
    }
}
impl From<CryptoError> for ManifestOpError {
    fn from(e: CryptoError) -> Self {
        ManifestOpError::Crypto(e)
    }
}
impl From<serde_json::Error> for ManifestOpError {
    fn from(e: serde_json::Error) -> Self {
        ManifestOpError::Serde(e)
    }
}
impl From<DeviceError> for ManifestOpError {
    fn from(e: DeviceError) -> Self {
        ManifestOpError::Device(e)
    }
}
impl From<VaultError> for ManifestOpError {
    fn from(e: VaultError) -> Self {
        ManifestOpError::Vault(e)
    }
}

/// This machine's local age identity for talking to a vault: its device
/// id, public key (goes in `devices.json`), and private key (never
/// uploaded). One identity per machine (not per-vault) -- Phase 2 doesn't
/// yet support a single machine juggling multiple independent vaults with
/// distinct identities; see `docs/08-open-questions-risks.md` if that
/// becomes a real need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalIdentity {
    pub device_id: String,
    pub public_key: String,
    pub private_key: String,
}

fn identity_store(state_dir: &Path) -> Result<SecretStore, ManifestOpError> {
    Ok(SecretStore::new(state_dir, DEVICE_IDENTITY_ENTROPY).map_err(VaultError::from)?)
}

/// Loads this machine's local identity, if `aam sync init`/`aam device
/// join` has already generated one under `state_dir`.
pub fn local_identity(state_dir: &Path) -> Result<Option<LocalIdentity>, ManifestOpError> {
    let store = identity_store(state_dir)?;
    let device_id = store.load(DEVICE_ID_KEY)?;
    let public_key = store.load(PUBLIC_KEY_KEY)?;
    let private_key = store.load(PRIVATE_KEY_KEY)?;
    match (device_id, public_key, private_key) {
        (Some(device_id), Some(public_key), Some(private_key)) => Ok(Some(LocalIdentity {
            device_id,
            public_key,
            private_key,
        })),
        _ => Ok(None),
    }
}

/// Loads this machine's local identity, generating and persisting a new
/// one under `state_dir` if none exists yet.
fn ensure_local_identity(state_dir: &Path) -> Result<LocalIdentity, ManifestOpError> {
    if let Some(identity) = local_identity(state_dir)? {
        return Ok(identity);
    }
    let (public_key, private_key) = generate_device_keypair();
    let identity = LocalIdentity {
        device_id: uuid::Uuid::new_v4().to_string(),
        public_key,
        private_key,
    };
    let store = identity_store(state_dir)?;
    store.save(DEVICE_ID_KEY, &identity.device_id)?;
    store.save(PUBLIC_KEY_KEY, &identity.public_key)?;
    store.save(PRIVATE_KEY_KEY, &identity.private_key)?;
    Ok(identity)
}

/// `04.3` for the very first device of a brand new vault: generates this
/// machine's identity (reusing one already on disk, if present), creates
/// `devices.json` with a fresh `vault_id` and this device as its sole
/// entry, encrypts it with `passphrase`, and pushes it. Errors with
/// [`ManifestOpError::AlreadyInitialized`] if a manifest already exists.
pub fn init_vault(
    backend: &impl SyncBackend,
    state_dir: &Path,
    passphrase: &str,
    label: &str,
) -> Result<DeviceEntry, ManifestOpError> {
    if backend.exists(DEVICES_MANIFEST_PATH)? {
        return Err(ManifestOpError::AlreadyInitialized);
    }

    let identity = ensure_local_identity(state_dir)?;
    let entry = DeviceEntry {
        device_id: identity.device_id,
        label: label.to_string(),
        age_public_key: identity.public_key,
        added_at: now_rfc3339(),
        revoked: false,
    };

    let manifest = DeviceManifest {
        vault_id: uuid::Uuid::new_v4().to_string(),
        devices: vec![entry.clone()],
    };
    let ciphertext = encrypt_with_passphrase(&manifest.to_json()?, passphrase)?;
    backend.put(DEVICES_MANIFEST_PATH, &ciphertext)?;
    Ok(entry)
}

/// `04.3` steps 1-5 for every device after the first: decrypts the
/// existing manifest with `passphrase`, generates (or reuses) this
/// machine's identity, appends it, and pushes the updated manifest.
///
/// **Does not** perform step 6 (re-encrypting existing blobs) -- see this
/// module's doc comment. After this call succeeds, the device is listed in
/// `devices.json` but cannot yet decrypt any existing blob; an already-
/// authorized device must run the domain-specific re-encrypt flow.
pub fn join_device_to_vault(
    backend: &impl SyncBackend,
    state_dir: &Path,
    passphrase: &str,
    label: &str,
) -> Result<DeviceEntry, ManifestOpError> {
    let ciphertext = backend
        .get(DEVICES_MANIFEST_PATH)?
        .ok_or(ManifestOpError::NotInitialized)?;
    let manifest = DeviceManifest::from_json(&decrypt_with_passphrase(&ciphertext, passphrase)?)?;

    let identity = ensure_local_identity(state_dir)?;
    let entry = DeviceEntry {
        device_id: identity.device_id,
        label: label.to_string(),
        age_public_key: identity.public_key,
        added_at: now_rfc3339(),
        revoked: false,
    };

    let next = join_device(&manifest, entry.clone())?;
    let ciphertext = encrypt_with_passphrase(&next.to_json()?, passphrase)?;
    backend.put(DEVICES_MANIFEST_PATH, &ciphertext)?;
    Ok(entry)
}

/// Decrypts and returns the current device manifest.
pub fn list_devices(
    backend: &impl SyncBackend,
    passphrase: &str,
) -> Result<DeviceManifest, ManifestOpError> {
    let ciphertext = backend
        .get(DEVICES_MANIFEST_PATH)?
        .ok_or(ManifestOpError::NotInitialized)?;
    Ok(DeviceManifest::from_json(&decrypt_with_passphrase(
        &ciphertext,
        passphrase,
    )?)?)
}

/// `04.4`: marks `device_id` revoked and pushes the updated manifest. Any
/// device holding the master passphrase can do this (layer 1 is
/// passphrase-gated, not per-device-key-gated) -- it does not require the
/// revoking device to itself be a recipient of any blob.
pub fn revoke_device_in_vault(
    backend: &impl SyncBackend,
    passphrase: &str,
    device_id: &str,
) -> Result<DeviceManifest, ManifestOpError> {
    let manifest = list_devices(backend, passphrase)?;
    let next = revoke_device(&manifest, device_id)?;
    let ciphertext = encrypt_with_passphrase(&next.to_json()?, passphrase)?;
    backend.put(DEVICES_MANIFEST_PATH, &ciphertext)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
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
                "aam-sync-manifest-test-{label}-{}-{unique}",
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
    fn init_vault_creates_a_single_device_manifest() {
        let backend_dir = TempDir::new("init-backend");
        let state_dir = TempDir::new("init-state");
        let backend = LocalDirBackend::new(&backend_dir.0);

        let entry = init_vault(&backend, &state_dir.0, "hunter2", "device-a").unwrap();
        assert_eq!(entry.label, "device-a");
        assert!(!entry.revoked);

        let manifest = list_devices(&backend, "hunter2").unwrap();
        assert_eq!(manifest.devices.len(), 1);
        assert_eq!(manifest.devices[0].device_id, entry.device_id);
    }

    #[test]
    fn init_vault_twice_errors() {
        let backend_dir = TempDir::new("init-twice-backend");
        let state_dir = TempDir::new("init-twice-state");
        let backend = LocalDirBackend::new(&backend_dir.0);

        init_vault(&backend, &state_dir.0, "hunter2", "device-a").unwrap();
        let err = init_vault(&backend, &state_dir.0, "hunter2", "device-a-again").unwrap_err();
        assert!(matches!(err, ManifestOpError::AlreadyInitialized));
    }

    #[test]
    fn join_device_to_vault_appends_a_second_device() {
        let backend_dir = TempDir::new("join-backend");
        let state_dir_a = TempDir::new("join-state-a");
        let state_dir_b = TempDir::new("join-state-b");
        let backend = LocalDirBackend::new(&backend_dir.0);

        init_vault(&backend, &state_dir_a.0, "hunter2", "device-a").unwrap();
        let entry_b = join_device_to_vault(&backend, &state_dir_b.0, "hunter2", "device-b").unwrap();

        let manifest = list_devices(&backend, "hunter2").unwrap();
        assert_eq!(manifest.devices.len(), 2);
        assert!(manifest.find(&entry_b.device_id).is_some());
    }

    #[test]
    fn join_device_to_vault_wrong_passphrase_fails() {
        let backend_dir = TempDir::new("join-wrong-pass-backend");
        let state_dir_a = TempDir::new("join-wrong-pass-state-a");
        let state_dir_b = TempDir::new("join-wrong-pass-state-b");
        let backend = LocalDirBackend::new(&backend_dir.0);

        init_vault(&backend, &state_dir_a.0, "hunter2", "device-a").unwrap();
        let err = join_device_to_vault(&backend, &state_dir_b.0, "wrong-pass", "device-b")
            .unwrap_err();
        assert!(matches!(err, ManifestOpError::Crypto(_)));
    }

    #[test]
    fn revoke_device_in_vault_marks_revoked() {
        let backend_dir = TempDir::new("revoke-backend");
        let state_dir_a = TempDir::new("revoke-state-a");
        let state_dir_b = TempDir::new("revoke-state-b");
        let backend = LocalDirBackend::new(&backend_dir.0);

        init_vault(&backend, &state_dir_a.0, "hunter2", "device-a").unwrap();
        let entry_b = join_device_to_vault(&backend, &state_dir_b.0, "hunter2", "device-b").unwrap();

        let manifest = revoke_device_in_vault(&backend, "hunter2", &entry_b.device_id).unwrap();
        assert!(manifest.find(&entry_b.device_id).unwrap().revoked);
        assert_eq!(manifest.active_recipients().len(), 1);
    }

    #[test]
    fn ensure_local_identity_is_stable_across_calls() {
        let state_dir = TempDir::new("stable-identity");
        let backend_dir_1 = TempDir::new("stable-identity-backend-1");
        let backend_dir_2 = TempDir::new("stable-identity-backend-2");
        let backend_1 = LocalDirBackend::new(&backend_dir_1.0);
        let backend_2 = LocalDirBackend::new(&backend_dir_2.0);

        // Same state_dir used against two different (unrelated) vaults --
        // the local identity persisted under state_dir should be reused,
        // not regenerated, each time.
        let entry_1 = init_vault(&backend_1, &state_dir.0, "pw1", "device-a").unwrap();
        let entry_2 = init_vault(&backend_2, &state_dir.0, "pw2", "device-a").unwrap();
        assert_eq!(entry_1.device_id, entry_2.device_id);
        assert_eq!(entry_1.age_public_key, entry_2.age_public_key);
    }

    #[test]
    fn list_devices_before_init_errors() {
        let backend_dir = TempDir::new("list-before-init");
        let backend = LocalDirBackend::new(&backend_dir.0);
        let err = list_devices(&backend, "hunter2").unwrap_err();
        assert!(matches!(err, ManifestOpError::NotInitialized));
    }
}
