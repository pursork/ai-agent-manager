//! Profile registry: the list of known `(tool, account)` Profiles and
//! where each one's config directory lives (`docs/03-credential-account-module.md`
//! §3.1/§3.6). Provider association is per-Profile (`Profile::provider`,
//! `None` = official subscription / default endpoint).

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Claude,
    Codex,
}

impl Tool {
    pub fn as_str(self) -> &'static str {
        match self {
            Tool::Claude => "claude",
            Tool::Codex => "codex",
        }
    }

    /// The environment variable this tool reads to redirect its entire
    /// state directory -- `docs/08-open-questions-risks.md` §8.1 confirms
    /// both are whole-directory redirects (`CLAUDE_CONFIG_DIR` community-
    /// verified, `CODEX_HOME` officially documented).
    pub fn config_dir_env_var(self) -> &'static str {
        match self {
            Tool::Claude => "CLAUDE_CONFIG_DIR",
            Tool::Codex => "CODEX_HOME",
        }
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub label: String,
    pub tool: Tool,
    pub config_dir: PathBuf,
    /// `Provider::id()` of a configured third-party/self-hosted endpoint,
    /// or `None` for the official subscription (`docs/03` §3.1's
    /// Account/Provider/Profile split -- a Profile pairs exactly one
    /// Account-equivalent config_dir with at most one Provider).
    pub provider: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    profiles: Vec<Profile>,
}

#[derive(Debug)]
pub enum RegistryError {
    Io(io::Error),
    Json(serde_json::Error),
    AlreadyExists { tool: Tool, label: String },
    NotFound { tool: Tool, label: String },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Io(e) => write!(f, "profile registry I/O error: {e}"),
            RegistryError::Json(e) => write!(f, "profile registry is corrupt (invalid JSON): {e}"),
            RegistryError::AlreadyExists { tool, label } => {
                write!(f, "a {tool} profile named '{label}' already exists")
            }
            RegistryError::NotFound { tool, label } => {
                write!(f, "no {tool} profile named '{label}' found")
            }
        }
    }
}

impl Error for RegistryError {}

impl From<io::Error> for RegistryError {
    fn from(e: io::Error) -> Self {
        RegistryError::Io(e)
    }
}

impl From<serde_json::Error> for RegistryError {
    fn from(e: serde_json::Error) -> Self {
        RegistryError::Json(e)
    }
}

/// The default location a newly-added Profile's config directory lives
/// at, if the caller doesn't pick their own: `~/.aam/profiles/<tool>/<label>/`.
pub fn default_config_dir_for(tool: Tool, label: &str) -> PathBuf {
    aam_core::aam_home()
        .join("profiles")
        .join(tool.as_str())
        .join(sanitize_label(label))
}

fn sanitize_label(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub struct ProfileRegistry {
    path: PathBuf,
}

impl ProfileRegistry {
    /// Opens the registry at `~/.aam/profiles.json` (or `$AAM_HOME/profiles.json`).
    pub fn open_default() -> Self {
        Self {
            path: aam_core::aam_home().join("profiles.json"),
        }
    }

    /// Opens a registry at an explicit path -- primarily for tests, so
    /// they never touch a real home directory.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn load(&self) -> Result<RegistryFile, RegistryError> {
        if !self.path.is_file() {
            return Ok(RegistryFile::default());
        }
        let text = fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&text)?)
    }

    fn save(&self, file: &RegistryFile) -> Result<(), RegistryError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(file)?;
        aam_core::atomic_write(&self.path, text.as_bytes())?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Profile>, RegistryError> {
        Ok(self.load()?.profiles)
    }

    pub fn list_for_tool(&self, tool: Tool) -> Result<Vec<Profile>, RegistryError> {
        Ok(self
            .load()?
            .profiles
            .into_iter()
            .filter(|p| p.tool == tool)
            .collect())
    }

    pub fn get(&self, tool: Tool, label: &str) -> Result<Option<Profile>, RegistryError> {
        Ok(self
            .load()?
            .profiles
            .into_iter()
            .find(|p| p.tool == tool && p.label == label))
    }

    pub fn add(&self, profile: Profile) -> Result<(), RegistryError> {
        let mut file = self.load()?;
        if file
            .profiles
            .iter()
            .any(|p| p.tool == profile.tool && p.label == profile.label)
        {
            return Err(RegistryError::AlreadyExists {
                tool: profile.tool,
                label: profile.label,
            });
        }
        file.profiles.push(profile);
        self.save(&file)
    }

    pub fn remove(&self, tool: Tool, label: &str) -> Result<(), RegistryError> {
        let mut file = self.load()?;
        let before = file.profiles.len();
        file.profiles.retain(|p| !(p.tool == tool && p.label == label));
        if file.profiles.len() == before {
            return Err(RegistryError::NotFound {
                tool,
                label: label.to_string(),
            });
        }
        self.save(&file)
    }

    pub fn set_provider(
        &self,
        tool: Tool,
        label: &str,
        provider: Option<String>,
    ) -> Result<(), RegistryError> {
        let mut file = self.load()?;
        let profile = file
            .profiles
            .iter_mut()
            .find(|p| p.tool == tool && p.label == label)
            .ok_or_else(|| RegistryError::NotFound {
                tool,
                label: label.to_string(),
            })?;
        profile.provider = provider;
        self.save(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_registry(label: &str) -> (ProfileRegistry, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "aam-switcher-registry-test-{label}-{}",
            std::process::id()
        ));
        let path = dir.join("profiles.json");
        (ProfileRegistry::open(&path), dir)
    }

    #[test]
    fn add_then_list_round_trips() {
        let (registry, dir) = temp_registry("add-list");
        registry
            .add(Profile {
                label: "官方账号1".into(),
                tool: Tool::Claude,
                config_dir: PathBuf::from("/tmp/whatever"),
                provider: None,
            })
            .unwrap();

        let profiles = registry.list().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].label, "官方账号1");
        assert_eq!(profiles[0].tool, Tool::Claude);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicate_label_within_same_tool_is_rejected() {
        let (registry, dir) = temp_registry("dup-same-tool");
        let make = |label: &str| Profile {
            label: label.into(),
            tool: Tool::Codex,
            config_dir: PathBuf::from("/tmp/x"),
            provider: None,
        };
        registry.add(make("a")).unwrap();
        let result = registry.add(make("a"));
        assert!(matches!(result, Err(RegistryError::AlreadyExists { .. })));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn same_label_allowed_across_different_tools() {
        let (registry, dir) = temp_registry("same-label-diff-tool");
        registry
            .add(Profile {
                label: "官方账号1".into(),
                tool: Tool::Claude,
                config_dir: PathBuf::from("/tmp/a"),
                provider: None,
            })
            .unwrap();
        registry
            .add(Profile {
                label: "官方账号1".into(),
                tool: Tool::Codex,
                config_dir: PathBuf::from("/tmp/b"),
                provider: None,
            })
            .unwrap();

        assert_eq!(registry.list().unwrap().len(), 2);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn remove_missing_profile_errors() {
        let (registry, dir) = temp_registry("remove-missing");
        let result = registry.remove(Tool::Claude, "does-not-exist");
        assert!(matches!(result, Err(RegistryError::NotFound { .. })));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn set_provider_updates_existing_profile() {
        let (registry, dir) = temp_registry("set-provider");
        registry
            .add(Profile {
                label: "a".into(),
                tool: Tool::Codex,
                config_dir: PathBuf::from("/tmp/a"),
                provider: None,
            })
            .unwrap();

        registry
            .set_provider(Tool::Codex, "a", Some("deepseek-v4-flash".into()))
            .unwrap();

        let profile = registry.get(Tool::Codex, "a").unwrap().unwrap();
        assert_eq!(profile.provider.as_deref(), Some("deepseek-v4-flash"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn persists_across_separate_registry_instances() {
        let (registry1, dir) = temp_registry("persist");
        registry1
            .add(Profile {
                label: "a".into(),
                tool: Tool::Claude,
                config_dir: PathBuf::from("/tmp/a"),
                provider: None,
            })
            .unwrap();

        let registry2 = ProfileRegistry::open(dir.join("profiles.json"));
        assert_eq!(registry2.list().unwrap().len(), 1);

        let _ = fs::remove_dir_all(dir);
    }
}
