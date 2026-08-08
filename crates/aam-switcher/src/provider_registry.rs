//! Provider configuration registry: where `aam provider add` persists a
//! third-party endpoint's non-secret settings (`docs/03` §3.1's
//! account/provider separation -- a Provider's config is independent of
//! any Profile, so it lives in its own small registry, not duplicated
//! per-Profile). The API key itself never lives here; see
//! [`crate::providers`]' construction helpers, which pull it from
//! `aam-vault` separately.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    Cpa,
    DeepseekV4Flash,
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderKind::Cpa => f.write_str("cpa"),
            ProviderKind::DeepseekV4Flash => f.write_str("deepseek-v4-flash"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    pub reasoning_effort: String,
    pub plan_reasoning_effort: String,
    pub supports_websockets: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    providers: Vec<ProviderRecord>,
}

#[derive(Debug)]
pub enum ProviderRegistryError {
    Io(io::Error),
    Json(serde_json::Error),
    AlreadyExists(String),
    NotFound(String),
}

impl fmt::Display for ProviderRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderRegistryError::Io(e) => write!(f, "provider registry I/O error: {e}"),
            ProviderRegistryError::Json(e) => write!(f, "provider registry is corrupt: {e}"),
            ProviderRegistryError::AlreadyExists(id) => write!(f, "a provider named '{id}' already exists"),
            ProviderRegistryError::NotFound(id) => write!(f, "no provider named '{id}' found"),
        }
    }
}

impl Error for ProviderRegistryError {}

impl From<io::Error> for ProviderRegistryError {
    fn from(e: io::Error) -> Self {
        ProviderRegistryError::Io(e)
    }
}
impl From<serde_json::Error> for ProviderRegistryError {
    fn from(e: serde_json::Error) -> Self {
        ProviderRegistryError::Json(e)
    }
}

pub struct ProviderRegistry {
    path: PathBuf,
}

impl ProviderRegistry {
    pub fn open_default() -> Self {
        Self {
            path: aam_core::aam_home().join("providers.json"),
        }
    }

    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn load(&self) -> Result<RegistryFile, ProviderRegistryError> {
        if !self.path.is_file() {
            return Ok(RegistryFile::default());
        }
        Ok(serde_json::from_str(&fs::read_to_string(&self.path)?)?)
    }

    fn save(&self, file: &RegistryFile) -> Result<(), ProviderRegistryError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        aam_core::atomic_write(&self.path, serde_json::to_string_pretty(file)?.as_bytes())?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<ProviderRecord>, ProviderRegistryError> {
        Ok(self.load()?.providers)
    }

    pub fn get(&self, id: &str) -> Result<Option<ProviderRecord>, ProviderRegistryError> {
        Ok(self.load()?.providers.into_iter().find(|p| p.id == id))
    }

    /// Adds a new record, or overwrites the existing one with the same
    /// `id` -- re-running `aam provider add` for the same id is how you
    /// update its settings, not an error.
    pub fn upsert(&self, record: ProviderRecord) -> Result<(), ProviderRegistryError> {
        let mut file = self.load()?;
        file.providers.retain(|p| p.id != record.id);
        file.providers.push(record);
        self.save(&file)
    }

    pub fn remove(&self, id: &str) -> Result<(), ProviderRegistryError> {
        let mut file = self.load()?;
        let before = file.providers.len();
        file.providers.retain(|p| p.id != id);
        if file.providers.len() == before {
            return Err(ProviderRegistryError::NotFound(id.to_string()));
        }
        self.save(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_registry(label: &str) -> (ProviderRegistry, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "aam-switcher-provider-registry-test-{label}-{}",
            std::process::id()
        ));
        (ProviderRegistry::open(dir.join("providers.json")), dir)
    }

    fn sample(id: &str) -> ProviderRecord {
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
    fn upsert_then_get_round_trips() {
        let (registry, dir) = temp_registry("upsert-get");
        registry.upsert(sample("cpa")).unwrap();
        let record = registry.get("cpa").unwrap().unwrap();
        assert_eq!(record.base_url, "https://cpa.example.com");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn upsert_replaces_existing_record_with_same_id() {
        let (registry, dir) = temp_registry("upsert-replace");
        registry.upsert(sample("cpa")).unwrap();
        let mut updated = sample("cpa");
        updated.base_url = "https://cpa-v2.example.com".into();
        registry.upsert(updated).unwrap();

        let records = registry.list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].base_url, "https://cpa-v2.example.com");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn remove_missing_provider_errors() {
        let (registry, dir) = temp_registry("remove-missing");
        assert!(matches!(
            registry.remove("nope"),
            Err(ProviderRegistryError::NotFound(_))
        ));
        let _ = fs::remove_dir_all(dir);
    }
}
