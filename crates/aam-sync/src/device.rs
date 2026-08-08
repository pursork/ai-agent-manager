//! Device manifest (`docs/04-webdav-sync-security.md` §4.2, layer 1): the
//! list of devices authorized to decrypt this vault's blobs. The manifest
//! itself is protected by the user's master passphrase (`devices.json.age`,
//! see [`crate::encrypt_with_passphrase`]); this module only deals with the
//! plaintext JSON shape and pure add/revoke edits -- no I/O.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

/// One entry in `devices.json` (`04.2`'s schema).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceEntry {
    pub device_id: String,
    pub label: String,
    pub age_public_key: String,
    /// RFC 3339 timestamp, e.g. `2026-08-08T12:00:00Z`.
    pub added_at: String,
    #[serde(default)]
    pub revoked: bool,
}

/// The full `devices.json` document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceManifest {
    pub vault_id: String,
    #[serde(default)]
    pub devices: Vec<DeviceEntry>,
}

impl DeviceManifest {
    pub fn new(vault_id: impl Into<String>) -> Self {
        Self {
            vault_id: vault_id.into(),
            devices: Vec::new(),
        }
    }

    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// The recipient list for layer-2 blob encryption (`04.2`): every
    /// non-revoked device's public key.
    pub fn active_recipients(&self) -> Vec<String> {
        self.devices
            .iter()
            .filter(|d| !d.revoked)
            .map(|d| d.age_public_key.clone())
            .collect()
    }

    pub fn find(&self, device_id: &str) -> Option<&DeviceEntry> {
        self.devices.iter().find(|d| d.device_id == device_id)
    }
}

#[derive(Debug)]
pub enum DeviceError {
    AlreadyExists(String),
    NotFound(String),
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::AlreadyExists(id) => write!(f, "device '{id}' is already in the manifest"),
            DeviceError::NotFound(id) => write!(f, "no device '{id}' in the manifest"),
        }
    }
}

impl Error for DeviceError {}

/// Adds `entry` to `manifest` (`04.3` steps 3-4), returning a new manifest.
/// Pure function: the caller re-encrypts and pushes the result themselves.
pub fn join_device(
    manifest: &DeviceManifest,
    entry: DeviceEntry,
) -> Result<DeviceManifest, DeviceError> {
    if manifest.devices.iter().any(|d| d.device_id == entry.device_id) {
        return Err(DeviceError::AlreadyExists(entry.device_id));
    }
    let mut next = manifest.clone();
    next.devices.push(entry);
    Ok(next)
}

/// Marks `device_id` revoked (`04.4`). Pure function; does not remove the
/// entry (keeping history of "this device was once authorized" is useful,
/// and `active_recipients` already excludes it from future encryption).
pub fn revoke_device(
    manifest: &DeviceManifest,
    device_id: &str,
) -> Result<DeviceManifest, DeviceError> {
    let mut next = manifest.clone();
    let device = next
        .devices
        .iter_mut()
        .find(|d| d.device_id == device_id)
        .ok_or_else(|| DeviceError::NotFound(device_id.to_string()))?;
    device.revoked = true;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(id: &str) -> DeviceEntry {
        DeviceEntry {
            device_id: id.to_string(),
            label: format!("device-{id}"),
            age_public_key: format!("age1{id}"),
            added_at: "2026-08-08T12:00:00Z".to_string(),
            revoked: false,
        }
    }

    #[test]
    fn join_device_adds_a_new_entry() {
        let manifest = DeviceManifest::new("vault-1");
        let next = join_device(&manifest, sample_entry("a")).unwrap();
        assert_eq!(next.devices.len(), 1);
        assert_eq!(next.devices[0].device_id, "a");
    }

    #[test]
    fn join_device_rejects_duplicate_id() {
        let manifest = DeviceManifest {
            vault_id: "vault-1".to_string(),
            devices: vec![sample_entry("a")],
        };
        let err = join_device(&manifest, sample_entry("a")).unwrap_err();
        assert!(matches!(err, DeviceError::AlreadyExists(id) if id == "a"));
    }

    #[test]
    fn revoke_device_marks_revoked_without_removing() {
        let manifest = DeviceManifest {
            vault_id: "vault-1".to_string(),
            devices: vec![sample_entry("a"), sample_entry("b")],
        };
        let next = revoke_device(&manifest, "a").unwrap();
        assert_eq!(next.devices.len(), 2);
        assert!(next.find("a").unwrap().revoked);
        assert!(!next.find("b").unwrap().revoked);
    }

    #[test]
    fn revoke_device_errors_on_unknown_id() {
        let manifest = DeviceManifest::new("vault-1");
        let err = revoke_device(&manifest, "ghost").unwrap_err();
        assert!(matches!(err, DeviceError::NotFound(id) if id == "ghost"));
    }

    #[test]
    fn active_recipients_excludes_revoked_devices() {
        let mut b = sample_entry("b");
        b.revoked = true;
        let manifest = DeviceManifest {
            vault_id: "vault-1".to_string(),
            devices: vec![sample_entry("a"), b],
        };
        assert_eq!(manifest.active_recipients(), vec!["age1a".to_string()]);
    }

    #[test]
    fn json_round_trip() {
        let manifest = DeviceManifest {
            vault_id: "vault-1".to_string(),
            devices: vec![sample_entry("a")],
        };
        let bytes = manifest.to_json().unwrap();
        let parsed = DeviceManifest::from_json(&bytes).unwrap();
        assert_eq!(parsed, manifest);
    }
}
