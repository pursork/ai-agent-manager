//! Account credential sync (`docs/04-webdav-sync-security.md` §4.10):
//! pushes/pulls just the single OS-level credential file each tool uses
//! for its official login state -- Claude's `.credentials.json`, Codex's
//! `auth.json` -- never the rest of `CLAUDE_CONFIG_DIR`/`CODEX_HOME`
//! (session history, cache, ...). Lives here, not in `aam-sync`, for the
//! same reason [`crate::provider_sync`] does: `aam-sync` must stay
//! domain-agnostic.
//!
//! The WebDAV blob key differs by tool (§4.10's asymmetry, confirmed by
//! inspecting real credential files rather than assumed): Claude's
//! `accessToken` carries no parseable identity claims, so its key is the
//! local Profile's `label`; Codex's `auth.json` JWTs do carry identity
//! claims, so its key is [`crate::codex_fingerprint::compute_fingerprint`],
//! stable across token refresh.

use crate::codex_fingerprint::{self, FingerprintError};
use crate::profile::{Profile, ProfileRegistry, RegistryError, Tool};
use crate::{claude_backend, codex_backend};
use aam_sync::{
    current_version, decrypt_with_passphrase, encrypt_with_passphrase, pull_if_newer,
    push_if_not_stale, BackendError, BlobMeta, ConflictError, CryptoError, SyncBackend,
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::PathBuf;

/// Well-known path for the account catalog (§4.10): since there is no
/// WebDAV directory listing (`04.8`), this small passphrase-protected
/// index (same protection tier as `devices.json.age`) is how `aam sync
/// list-accounts` can show a human what's pushable before they guess a
/// `pull-account --key`.
const ACCOUNTS_CATALOG_PATH: &str = "accounts.json.age";

#[derive(Debug)]
pub enum AccountSyncError {
    Io(std::io::Error),
    NoCredentialFile(PathBuf),
    Backend(BackendError),
    Sync(ConflictError),
    Crypto(CryptoError),
    Serde(serde_json::Error),
    Fingerprint(FingerprintError),
    Registry(RegistryError),
    NotFound { tool: Tool, key: String },
}

impl fmt::Display for AccountSyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccountSyncError::Io(e) => write!(f, "I/O error: {e}"),
            AccountSyncError::NoCredentialFile(p) => write!(
                f,
                "no credential file at {} -- log in with this Profile first (`aam claude/codex <label>`)",
                p.display()
            ),
            AccountSyncError::Backend(e) => write!(f, "{e}"),
            AccountSyncError::Sync(e) => write!(f, "{e}"),
            AccountSyncError::Crypto(e) => write!(f, "{e}"),
            AccountSyncError::Serde(e) => write!(f, "account catalog is corrupt: {e}"),
            AccountSyncError::Fingerprint(e) => write!(f, "{e}"),
            AccountSyncError::Registry(e) => write!(f, "{e}"),
            AccountSyncError::NotFound { tool, key } => {
                write!(f, "no {tool} account blob found for key '{key}' at this vault")
            }
        }
    }
}

impl Error for AccountSyncError {}

impl From<std::io::Error> for AccountSyncError {
    fn from(e: std::io::Error) -> Self {
        AccountSyncError::Io(e)
    }
}
impl From<BackendError> for AccountSyncError {
    fn from(e: BackendError) -> Self {
        AccountSyncError::Backend(e)
    }
}
impl From<ConflictError> for AccountSyncError {
    fn from(e: ConflictError) -> Self {
        AccountSyncError::Sync(e)
    }
}
impl From<CryptoError> for AccountSyncError {
    fn from(e: CryptoError) -> Self {
        AccountSyncError::Crypto(e)
    }
}
impl From<serde_json::Error> for AccountSyncError {
    fn from(e: serde_json::Error) -> Self {
        AccountSyncError::Serde(e)
    }
}
impl From<FingerprintError> for AccountSyncError {
    fn from(e: FingerprintError) -> Self {
        AccountSyncError::Fingerprint(e)
    }
}
impl From<RegistryError> for AccountSyncError {
    fn from(e: RegistryError) -> Self {
        AccountSyncError::Registry(e)
    }
}

/// One entry in the account catalog (§4.10). `email_hint` is best-effort
/// (only ever populated for Codex, whose JWTs carry it -- Claude has
/// nothing equivalent to offer) and is exactly that, a *hint*: never
/// treated as an identity guarantee anywhere else in this crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountCatalogEntry {
    pub tool: String,
    pub key: String,
    pub label_hint: String,
    pub email_hint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct AccountCatalog {
    #[serde(default)]
    entries: Vec<AccountCatalogEntry>,
}

/// Which file, relative to a Profile's `config_dir`, holds that tool's
/// official login credential -- confirmed against the real files on this
/// machine while designing §4.10, not assumed.
pub fn credential_file_name(tool: Tool) -> &'static str {
    match tool {
        Tool::Claude => ".credentials.json",
        Tool::Codex => "auth.json",
    }
}

fn credential_file_path(profile: &Profile) -> PathBuf {
    profile.config_dir.join(credential_file_name(profile.tool))
}

fn blob_path_for(tool: Tool, key: &str) -> String {
    format!("credentials/{}/{key}.blob.age", tool.as_str())
}

fn read_catalog(backend: &impl SyncBackend, passphrase: &str) -> Result<AccountCatalog, AccountSyncError> {
    match backend.get(ACCOUNTS_CATALOG_PATH)? {
        None => Ok(AccountCatalog::default()),
        Some(bytes) => Ok(serde_json::from_slice(&decrypt_with_passphrase(
            &bytes, passphrase,
        )?)?),
    }
}

fn write_catalog(
    backend: &impl SyncBackend,
    passphrase: &str,
    catalog: &AccountCatalog,
) -> Result<(), AccountSyncError> {
    let plaintext = serde_json::to_vec_pretty(catalog)?;
    let ciphertext = encrypt_with_passphrase(&plaintext, passphrase)?;
    backend.put(ACCOUNTS_CATALOG_PATH, &ciphertext)?;
    Ok(())
}

fn upsert_catalog_entry(
    backend: &impl SyncBackend,
    passphrase: &str,
    entry: AccountCatalogEntry,
) -> Result<(), AccountSyncError> {
    let mut catalog = read_catalog(backend, passphrase)?;
    catalog
        .entries
        .retain(|e| !(e.tool == entry.tool && e.key == entry.key));
    catalog.entries.push(entry);
    write_catalog(backend, passphrase, &catalog)
}

/// Decrypts and returns the account catalog (§4.10) -- what `aam sync
/// list-accounts` shows before a `pull-account`.
pub fn list_accounts(
    backend: &impl SyncBackend,
    passphrase: &str,
) -> Result<Vec<AccountCatalogEntry>, AccountSyncError> {
    Ok(read_catalog(backend, passphrase)?.entries)
}

/// Pushes `profile`'s credential file, encrypted to `recipients`, and
/// updates the account catalog. Reads the remote blob's current version
/// immediately before pushing, same as [`crate::provider_sync::push_provider`].
pub fn push_account(
    backend: &impl SyncBackend,
    profile: &Profile,
    recipients: &[String],
    device_id: &str,
    passphrase: &str,
) -> Result<BlobMeta, AccountSyncError> {
    let path = credential_file_path(profile);
    let bytes = fs::read(&path).map_err(|_| AccountSyncError::NoCredentialFile(path.clone()))?;

    let (key, email_hint) = match profile.tool {
        Tool::Claude => (profile.label.clone(), None),
        Tool::Codex => {
            let identity = codex_fingerprint::extract_identity(&bytes)?;
            let hint = (!identity.email.is_empty()).then_some(identity.email);
            (identity.fingerprint, hint)
        }
    };

    let blob_path = blob_path_for(profile.tool, &key);
    let base_version = current_version(backend, &blob_path)?;
    let meta = push_if_not_stale(backend, &blob_path, &bytes, recipients, device_id, base_version)?;

    upsert_catalog_entry(
        backend,
        passphrase,
        AccountCatalogEntry {
            tool: profile.tool.as_str().to_string(),
            key,
            label_hint: profile.label.clone(),
            email_hint,
        },
    )?;

    Ok(meta)
}

/// Pulls the credential blob at `(tool, key)`, creating a local Profile
/// named `local_label` if none exists yet for `tool`, and writes the
/// decrypted bytes into its credential file. This is the step that makes
/// "device B, never logged in, gets to use the official subscription"
/// (`00-overview.md`'s acceptance scenario step 7) actually work end to
/// end, rather than leaving the user to `profile add` by hand afterwards.
pub fn pull_account(
    backend: &impl SyncBackend,
    registry: &ProfileRegistry,
    tool: Tool,
    key: &str,
    local_label: &str,
    identity: &str,
) -> Result<Profile, AccountSyncError> {
    let blob_path = blob_path_for(tool, key);
    let blob = pull_if_newer(backend, &blob_path, identity, 0)?.ok_or_else(|| {
        AccountSyncError::NotFound {
            tool,
            key: key.to_string(),
        }
    })?;

    let profile = match registry.get(tool, local_label)? {
        Some(existing) => existing,
        None => match tool {
            Tool::Claude => claude_backend::create_profile(registry, local_label)
                .map_err(|e| AccountSyncError::Io(std::io::Error::other(e.to_string())))?,
            Tool::Codex => codex_backend::create_profile(registry, local_label)
                .map_err(|e| AccountSyncError::Io(std::io::Error::other(e.to_string())))?,
        },
    };

    let cred_path = credential_file_path(&profile);
    aam_core::atomic_write(&cred_path, &blob.plaintext)?;

    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileRegistry;
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
                "aam-switcher-account-sync-test-{label}-{}-{unique}",
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
                "aam-switcher-account-sync-home-{label}-{}-{unique}",
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

    fn claude_profile_with_credentials(registry: &ProfileRegistry, label: &str, contents: &[u8]) -> Profile {
        let profile = claude_backend::create_profile(registry, label).unwrap();
        fs::write(credential_file_path(&profile), contents).unwrap();
        profile
    }

    #[test]
    fn push_then_pull_claude_account_round_trips_by_label() {
        let _home = AamHomeGuard::new("claude-roundtrip");
        let backend_dir = TempDir::new("claude-roundtrip-backend");
        let backend = LocalDirBackend::new(&backend_dir.0);
        let (public, private) = generate_device_keypair();

        // Two separate registry files simulate two separate devices (both
        // still under one AAM_HOME here since this test doesn't need real
        // process-level isolation, just "device B's registry starts empty").
        let registry_a = ProfileRegistry::open(_home.dir.join("device-a-profiles.json"));
        let profile_a =
            claude_profile_with_credentials(&registry_a, "work", br#"{"claudeAiOauth":{"accessToken":"tok-a"}}"#);

        let meta = push_account(&backend, &profile_a, &[public], "device-a", "passphrase").unwrap();
        assert_eq!(meta.version, 1);

        let catalog = list_accounts(&backend, "passphrase").unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].tool, "claude");
        assert_eq!(catalog[0].key, "work");
        assert_eq!(catalog[0].email_hint, None);

        // Simulate device B: no local "work" Profile yet.
        let registry_b = ProfileRegistry::open(_home.dir.join("device-b-profiles.json"));
        assert!(registry_b.get(Tool::Claude, "work").unwrap().is_none());

        let pulled_profile =
            pull_account(&backend, &registry_b, Tool::Claude, "work", "work", &private).unwrap();
        assert_eq!(pulled_profile.label, "work");

        let written = fs::read(credential_file_path(&pulled_profile)).unwrap();
        assert_eq!(written, br#"{"claudeAiOauth":{"accessToken":"tok-a"}}"#);
    }

    #[test]
    fn push_without_credential_file_errors() {
        let _home = AamHomeGuard::new("no-cred-file");
        let backend_dir = TempDir::new("no-cred-file-backend");
        let backend = LocalDirBackend::new(&backend_dir.0);
        let (public, _private) = generate_device_keypair();

        let registry = ProfileRegistry::open_default();
        let profile = claude_backend::create_profile(&registry, "empty").unwrap();
        // No credential file written -- this Profile was never logged in.

        let err = push_account(&backend, &profile, &[public], "device-a", "passphrase").unwrap_err();
        assert!(matches!(err, AccountSyncError::NoCredentialFile(_)));
    }

    #[test]
    fn pull_of_unknown_key_errors_not_found() {
        let _home = AamHomeGuard::new("pull-unknown");
        let backend_dir = TempDir::new("pull-unknown-backend");
        let backend = LocalDirBackend::new(&backend_dir.0);
        let (_public, private) = generate_device_keypair();

        let registry = ProfileRegistry::open_default();
        let err = pull_account(&backend, &registry, Tool::Claude, "ghost", "ghost", &private)
            .unwrap_err();
        assert!(matches!(err, AccountSyncError::NotFound { .. }));
    }

    #[test]
    fn pull_reuses_an_existing_local_profile_instead_of_erroring() {
        let _home = AamHomeGuard::new("pull-existing");
        let backend_dir = TempDir::new("pull-existing-backend");
        let backend = LocalDirBackend::new(&backend_dir.0);
        let (public, private) = generate_device_keypair();

        let registry = ProfileRegistry::open_default();
        let profile =
            claude_profile_with_credentials(&registry, "work", br#"{"claudeAiOauth":{"accessToken":"old"}}"#);
        push_account(&backend, &profile, &[public], "device-a", "passphrase").unwrap();

        // Overwrite locally (simulating staleness), then pull should
        // refresh the same Profile's credential file, not create a
        // second one.
        fs::write(credential_file_path(&profile), b"stale").unwrap();
        let pulled = pull_account(&backend, &registry, Tool::Claude, "work", "work", &private).unwrap();

        assert_eq!(registry.list().unwrap().len(), 1);
        let written = fs::read(credential_file_path(&pulled)).unwrap();
        assert_eq!(written, br#"{"claudeAiOauth":{"accessToken":"old"}}"#);
    }
}
