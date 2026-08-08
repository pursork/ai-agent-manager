//! WebDAV encrypted sync engine (`docs/04-webdav-sync-security.md`).
//!
//! This crate is deliberately domain-agnostic: it knows how to talk to a
//! storage backend, encrypt/decrypt with `age`, manage a device manifest,
//! and resolve version conflicts on versioned blobs -- but it has no idea
//! what a "Provider" or "Profile" is. Domain-specific wiring (e.g.
//! "push `aam_switcher::ProviderRecord` to `providers/<id>.blob.age`")
//! lives in the crates that already depend on both `aam-sync` and the
//! domain crate (`aam-switcher`, per `docs/02-architecture.md`'s
//! dependency graph), not here -- `aam-sync` must not depend back on
//! `aam-switcher`, or the crate graph would cycle.

mod age_crypto;
mod backend;
mod blob;
mod device;
mod manifest_ops;
mod util;

pub use age_crypto::{
    decrypt_multi_recipient, decrypt_with_passphrase, encrypt_multi_recipient,
    encrypt_with_passphrase, generate_device_keypair, CryptoError,
};
pub use backend::{BackendError, LocalDirBackend, SyncBackend, WebDavBackend};
pub use blob::{
    current_version, pull_if_newer, push_if_not_stale, BlobMeta, ConflictError, VersionedBlob,
};
pub use device::{join_device, revoke_device, DeviceEntry, DeviceError, DeviceManifest};
pub use manifest_ops::{
    init_vault, join_device_to_vault, list_devices, local_identity, revoke_device_in_vault,
    LocalIdentity, ManifestOpError, DEVICES_MANIFEST_PATH,
};
