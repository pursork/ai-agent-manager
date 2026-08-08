//! Wires `aam-sync`'s generic primitives to this crate's `ProviderRegistry`
//! and `aam-vault`-stored API keys (`docs/04-webdav-sync-security.md`
//! §4.5's `providers/<id>.blob.age`). This module -- not `aam-sync` itself
//! -- is where "a Provider's config is a thing that gets synced" is known,
//! because `aam-sync` must stay domain-agnostic (see its crate-level doc
//! comment): `aam-switcher` already depends on both `aam-sync` and
//! `aam-vault`, so this is the right layer for the wiring.
//!
//! Phase 2's first cut only syncs Provider *configuration* (`ProviderRecord`
//! and its API key), not account credentials (Claude/Codex login state),
//! which is out of scope here (`docs/08-open-questions-risks.md` #15).

use crate::provider_registry::{ProviderRecord, ProviderRegistry, ProviderRegistryError};
use crate::provider_secret_store;
use aam_sync::{current_version, pull_if_newer, push_if_not_stale, BlobMeta, ConflictError, SyncBackend};
use aam_vault::VaultError;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

#[derive(Debug, Serialize, Deserialize)]
struct ProviderBlobPayload {
    record: ProviderRecord,
    api_key: String,
}

#[derive(Debug)]
pub enum ProviderSyncError {
    Registry(ProviderRegistryError),
    Vault(VaultError),
    /// `provider_secret_store()`'s own directory couldn't be opened --
    /// distinct from `Vault`, which is a `save`/`load` failure once open.
    VaultInit(std::io::Error),
    Sync(ConflictError),
    Serde(serde_json::Error),
    NotFound(String),
    NoApiKey(String),
}

impl fmt::Display for ProviderSyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderSyncError::Registry(e) => write!(f, "{e}"),
            ProviderSyncError::Vault(e) => write!(f, "{e}"),
            ProviderSyncError::VaultInit(e) => write!(f, "opening local provider-secret store: {e}"),
            ProviderSyncError::Sync(e) => write!(f, "{e}"),
            ProviderSyncError::Serde(e) => write!(f, "provider blob is corrupt: {e}"),
            ProviderSyncError::NotFound(id) => write!(f, "no provider named '{id}' registered locally"),
            ProviderSyncError::NoApiKey(id) => write!(f, "no API key saved locally for provider '{id}'"),
        }
    }
}

impl Error for ProviderSyncError {}

impl From<ProviderRegistryError> for ProviderSyncError {
    fn from(e: ProviderRegistryError) -> Self {
        ProviderSyncError::Registry(e)
    }
}
impl From<VaultError> for ProviderSyncError {
    fn from(e: VaultError) -> Self {
        ProviderSyncError::Vault(e)
    }
}
impl From<ConflictError> for ProviderSyncError {
    fn from(e: ConflictError) -> Self {
        ProviderSyncError::Sync(e)
    }
}
impl From<serde_json::Error> for ProviderSyncError {
    fn from(e: serde_json::Error) -> Self {
        ProviderSyncError::Serde(e)
    }
}

/// `docs/04-webdav-sync-security.md` §4.5's path for one provider's blob.
pub fn blob_path_for(provider_id: &str) -> String {
    format!("providers/{provider_id}.blob.age")
}

fn secret_store() -> Result<aam_vault::SecretStore, ProviderSyncError> {
    provider_secret_store().map_err(ProviderSyncError::VaultInit)
}

/// Pushes `provider_id`'s config + API key, encrypted to `recipients`.
/// Reads the remote blob's current version immediately before pushing
/// (rather than trusting a locally cached version number) so the
/// optimistic-concurrency check in `push_if_not_stale` is checked against
/// genuinely fresh state.
pub fn push_provider(
    backend: &impl SyncBackend,
    registry: &ProviderRegistry,
    provider_id: &str,
    recipients: &[String],
    device_id: &str,
) -> Result<BlobMeta, ProviderSyncError> {
    let record = registry
        .get(provider_id)?
        .ok_or_else(|| ProviderSyncError::NotFound(provider_id.to_string()))?;
    let api_key = secret_store()?
        .load(provider_id)?
        .ok_or_else(|| ProviderSyncError::NoApiKey(provider_id.to_string()))?;

    let plaintext = serde_json::to_vec(&ProviderBlobPayload { record, api_key })?;
    let path = blob_path_for(provider_id);
    let base_version = current_version(backend, &path)?;
    Ok(push_if_not_stale(
        backend,
        &path,
        &plaintext,
        recipients,
        device_id,
        base_version,
    )?)
}

/// Pulls `provider_id`'s config + API key (if a blob exists) and writes
/// them into the local `ProviderRegistry` + `aam-vault`. Returns `None` if
/// no blob exists yet at that path.
pub fn pull_provider(
    backend: &impl SyncBackend,
    registry: &ProviderRegistry,
    provider_id: &str,
    identity: &str,
) -> Result<Option<BlobMeta>, ProviderSyncError> {
    let path = blob_path_for(provider_id);
    let Some(blob) = pull_if_newer(backend, &path, identity, 0)? else {
        return Ok(None);
    };
    let payload: ProviderBlobPayload = serde_json::from_slice(&blob.plaintext)?;
    registry.upsert(payload.record)?;
    secret_store()?.save(provider_id, &payload.api_key)?;
    Ok(Some(blob.meta))
}

/// `04.3` step 6's manual re-encrypt, for one provider: pulls the current
/// blob with `my_identity` and re-pushes it encrypted to `new_recipients`
/// (typically the full, updated `devices.json` active-recipient list after
/// a device joined). Returns `None` if this provider has no blob yet
/// (nothing to re-encrypt).
pub fn reencrypt_provider(
    backend: &impl SyncBackend,
    provider_id: &str,
    my_identity: &str,
    new_recipients: &[String],
    device_id: &str,
) -> Result<Option<BlobMeta>, ProviderSyncError> {
    let path = blob_path_for(provider_id);
    let Some(blob) = pull_if_newer(backend, &path, my_identity, 0)? else {
        return Ok(None);
    };
    Ok(Some(push_if_not_stale(
        backend,
        &path,
        &blob.plaintext,
        new_recipients,
        device_id,
        blob.meta.version,
    )?))
}

/// Re-encrypts every provider this device's local registry knows about.
/// **Known limitation** (documented, not silently papered over): without
/// WebDAV directory listing (`docs/04-webdav-sync-security.md` §4.8
/// deliberately doesn't use PROPFIND), this can only re-encrypt providers
/// already known to *this* device's local registry -- a provider some
/// other device pushed that this device never pulled won't be found. Run
/// `aam sync pull` for every known provider id before `reencrypt` to
/// minimize that gap.
pub fn reencrypt_all_known_providers(
    backend: &impl SyncBackend,
    registry: &ProviderRegistry,
    my_identity: &str,
    new_recipients: &[String],
    device_id: &str,
) -> Result<Vec<(String, Option<BlobMeta>)>, ProviderSyncError> {
    let mut results = Vec::new();
    for record in registry.list()? {
        let meta = reencrypt_provider(backend, &record.id, my_identity, new_recipients, device_id)?;
        results.push((record.id, meta));
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_registry::ProviderKind;
    use aam_sync::{generate_device_keypair, LocalDirBackend};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "aam-switcher-provider-sync-test-{label}-{}-{unique}",
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

    /// These tests exercise the real `aam-vault::SecretStore` via
    /// `provider_secret_store()`, which is rooted at `aam_core::aam_home()`
    /// -- point `AAM_HOME` at a throwaway directory so they don't touch the
    /// developer's real `~/.aam`. Holds the crate-wide
    /// `crate::test_support::AAM_HOME_ENV_LOCK` for its entire lifetime so
    /// this doesn't race with `codex.rs`'s tests, which mutate the same
    /// env var.
    struct AamHomeGuard {
        dir: PathBuf,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl AamHomeGuard {
        fn new(label: &str) -> Self {
            let lock = crate::test_support::AAM_HOME_ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "aam-switcher-provider-sync-home-{label}-{}-{unique}",
                std::process::id()
            ));
            std::env::set_var("AAM_HOME", &dir);
            AamHomeGuard { dir, _lock: lock }
        }
    }

    impl Drop for AamHomeGuard {
        fn drop(&mut self) {
            std::env::remove_var("AAM_HOME");
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn sample_record(id: &str) -> ProviderRecord {
        ProviderRecord {
            id: id.to_string(),
            kind: ProviderKind::Cpa,
            base_url: "https://cpa.example.com".into(),
            model: "gpt-5".into(),
            reasoning_effort: "high".into(),
            plan_reasoning_effort: "high".into(),
            supports_websockets: false,
        }
    }

    #[test]
    fn push_then_pull_round_trips_config_and_key() {
        // Serialized: mutates the process-wide AAM_HOME env var.
        let _home = AamHomeGuard::new("roundtrip");
        let backend_dir = TempDir::new("roundtrip-backend");
        let backend = LocalDirBackend::new(&backend_dir.0);
        let (public, private) = generate_device_keypair();

        let registry_a = ProviderRegistry::open_default();
        registry_a.upsert(sample_record("cpa")).unwrap();
        provider_secret_store().unwrap().save("cpa", "sk-test-key").unwrap();

        let meta = push_provider(&backend, &registry_a, "cpa", &[public], "device-a").unwrap();
        assert_eq!(meta.version, 1);

        // Simulate a second device: same AAM_HOME in this test (a real
        // second device would have its own), but a fresh in-memory
        // registry view -- what matters is pull_provider repopulates it
        // from the blob, not from local state.
        let registry_b = ProviderRegistry::open_default();
        let pulled_meta = pull_provider(&backend, &registry_b, "cpa", &private)
            .unwrap()
            .expect("blob should exist");
        assert_eq!(pulled_meta.version, 1);

        let record = registry_b.get("cpa").unwrap().unwrap();
        assert_eq!(record.base_url, "https://cpa.example.com");
        let key = provider_secret_store().unwrap().load("cpa").unwrap();
        assert_eq!(key.as_deref(), Some("sk-test-key"));
    }

    #[test]
    fn push_without_local_api_key_errors() {
        let _home = AamHomeGuard::new("no-key");
        let backend_dir = TempDir::new("no-key-backend");
        let backend = LocalDirBackend::new(&backend_dir.0);
        let (public, _private) = generate_device_keypair();

        let registry = ProviderRegistry::open_default();
        registry.upsert(sample_record("cpa")).unwrap();

        let err = push_provider(&backend, &registry, "cpa", &[public], "device-a").unwrap_err();
        assert!(matches!(err, ProviderSyncError::NoApiKey(id) if id == "cpa"));
    }

    #[test]
    fn reencrypt_adds_a_new_recipient_without_changing_the_payload() {
        let _home = AamHomeGuard::new("reencrypt");
        let backend_dir = TempDir::new("reencrypt-backend");
        let backend = LocalDirBackend::new(&backend_dir.0);
        let (public_a, private_a) = generate_device_keypair();
        let (public_b, private_b) = generate_device_keypair();

        let registry = ProviderRegistry::open_default();
        registry.upsert(sample_record("cpa")).unwrap();
        provider_secret_store().unwrap().save("cpa", "sk-test-key").unwrap();

        push_provider(&backend, &registry, "cpa", std::slice::from_ref(&public_a), "device-a")
            .unwrap();

        // Device B was just added to devices.json but isn't yet a
        // recipient of this blob -- it can't decrypt it.
        assert!(pull_provider(&backend, &registry, "cpa", &private_b).is_err());

        let meta = reencrypt_provider(
            &backend,
            "cpa",
            &private_a,
            &[public_a, public_b],
            "device-a",
        )
        .unwrap()
        .expect("blob should have existed to re-encrypt");
        assert_eq!(meta.version, 2);

        // Now device B can decrypt it.
        let pulled = pull_provider(&backend, &registry, "cpa", &private_b)
            .unwrap()
            .expect("blob should exist");
        assert_eq!(pulled.version, 2);
    }
}
