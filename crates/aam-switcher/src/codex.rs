//! Codex backend: the "N directories" account model (`CODEX_HOME`
//! selection at launch, `docs/03` §3.2) plus the Provider-application
//! standard operation sequence (`03.5`, mirroring `codex-skill`'s
//! `Switch-ToProviderMode`).
//!
//! No Skills provisioning step here (unlike `claude.rs`): `08.1` confirmed
//! Codex's Skills always come from `$HOME/.agents/skills`, independent of
//! `CODEX_HOME`, so there is nothing to keep in sync per-Profile.

use crate::profile::{default_config_dir_for, Profile, ProfileRegistry, RegistryError, Tool};
use crate::provider::{Provider, ProviderConfig, ToolKind, VerifyError};
use crate::token_helper;
use aam_core::TransactionalOp;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum CodexBackendError {
    Registry(RegistryError),
    Io(io::Error),
    Vault(aam_vault::VaultError),
    Verify(VerifyError),
}

impl fmt::Display for CodexBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodexBackendError::Registry(e) => write!(f, "{e}"),
            CodexBackendError::Io(e) => write!(f, "I/O error: {e}"),
            CodexBackendError::Vault(e) => write!(f, "{e}"),
            CodexBackendError::Verify(e) => write!(f, "{e}"),
        }
    }
}

impl Error for CodexBackendError {}

impl From<RegistryError> for CodexBackendError {
    fn from(e: RegistryError) -> Self {
        CodexBackendError::Registry(e)
    }
}
impl From<io::Error> for CodexBackendError {
    fn from(e: io::Error) -> Self {
        CodexBackendError::Io(e)
    }
}
impl From<aam_vault::VaultError> for CodexBackendError {
    fn from(e: aam_vault::VaultError) -> Self {
        CodexBackendError::Vault(e)
    }
}

/// Creates a new Codex Profile: just a fresh `CODEX_HOME`-equivalent
/// directory, registered in `registry`. Actually logging in
/// (`codex login`) is left to the user, run against this directory via
/// `aam codex <label>` -- this project never drives the OAuth flow itself
/// (`docs/03` §3.6, matching `codex-skill`'s own stance).
pub fn create_profile(registry: &ProfileRegistry, label: &str) -> Result<Profile, CodexBackendError> {
    let config_dir = default_config_dir_for(Tool::Codex, label);
    fs::create_dir_all(&config_dir)?;

    let profile = Profile {
        label: label.to_string(),
        tool: Tool::Codex,
        config_dir,
        provider: None,
    };
    registry.add(profile.clone())?;
    Ok(profile)
}

/// Environment variables to set when launching `codex` under this Profile.
pub fn launch_env(profile: &Profile) -> Vec<(String, String)> {
    vec![(
        Tool::Codex.config_dir_env_var().to_string(),
        profile.config_dir.display().to_string(),
    )]
}

/// Runs `codex login status` under this Profile's `CODEX_HOME` and
/// reports whether it's logged in. Mirrors `codex-skill`'s
/// `Test-CodexLoginStatus`: success requires both exit code 0 *and* the
/// combined stdout+stderr containing "Logged in using ChatGPT"
/// (case-insensitive) -- a 0 exit code alone isn't treated as sufficient.
pub fn verify_login(profile: &Profile) -> io::Result<bool> {
    let output = std::process::Command::new("codex")
        .args(["login", "status"])
        .env(Tool::Codex.config_dir_env_var(), &profile.config_dir)
        .output()?;

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output.status.success() && combined.to_lowercase().contains("logged in using chatgpt"))
}

/// A rollback snapshot: for each file this operation might touch, either
/// its previous content (`Some`) or confirmation it didn't exist
/// (`None`) -- restoring means writing the content back or deleting it.
pub struct CodexProviderBackup {
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
}

/// Applies a Provider to a Codex Profile: writes `config.toml`, the
/// command-backed bearer-token helper (Windows) and its encrypted token
/// file, and any Provider-specific extra files (e.g. DeepSeek's model
/// catalog) -- following the same snapshot -> apply -> verify -> rollback
/// sequence as `codex-skill`'s `Switch-ToProviderMode` (`03.5`).
pub struct ApplyCodexProvider<'p> {
    config_dir: PathBuf,
    provider: &'p dyn Provider,
    materialized: ProviderConfig,
}

impl<'p> ApplyCodexProvider<'p> {
    pub fn new(config_dir: PathBuf, provider: &'p dyn Provider) -> Self {
        let materialized = provider.materialize(ToolKind::Codex {
            config_dir: config_dir.clone(),
        });
        Self {
            config_dir,
            provider,
            materialized,
        }
    }

    fn managed_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.config_dir.join("config.toml")];
        if let Some(token_file) = &self.materialized.codex_token_file {
            paths.push(token_file.clone());
        }
        paths.push(token_helper::helper_script_path(&self.config_dir));
        for (path, _content) in &self.materialized.codex_extra_files {
            paths.push(path.clone());
        }
        paths
    }
}

impl<'p> TransactionalOp for ApplyCodexProvider<'p> {
    type Snapshot = CodexProviderBackup;
    type Error = CodexBackendError;

    fn snapshot(&self) -> Result<CodexProviderBackup, CodexBackendError> {
        let mut files = Vec::new();
        for path in self.managed_paths() {
            let existing = if path.is_file() {
                Some(fs::read(&path)?)
            } else {
                None
            };
            files.push((path, existing));
        }
        Ok(CodexProviderBackup { files })
    }

    fn apply(&mut self) -> Result<(), CodexBackendError> {
        fs::create_dir_all(&self.config_dir)?;

        // Encrypted (Windows) / chmod-600 (Unix) token file, via aam-vault
        // -- entropy must match token_helper::TOKEN_ENTROPY exactly so the
        // generated helper script (Windows) can decrypt it.
        let store = aam_vault::SecretStore::new(&self.config_dir, token_helper::TOKEN_ENTROPY)?;
        store.save("provider-token", self.provider.api_key())?;

        #[cfg(windows)]
        if let Some(script) = &self.materialized.codex_token_helper_script {
            aam_core::atomic_write(&token_helper::helper_script_path(&self.config_dir), script.as_bytes())?;
        }

        for (path, content) in &self.materialized.codex_extra_files {
            aam_core::atomic_write(path, content)?;
        }

        if let Some(toml) = &self.materialized.codex_config_toml {
            aam_core::atomic_write(&self.config_dir.join("config.toml"), toml.as_bytes())?;
        }

        Ok(())
    }

    fn verify(&self) -> Result<(), CodexBackendError> {
        self.provider
            .verify(&self.materialized)
            .map_err(CodexBackendError::Verify)
    }

    fn rollback(&mut self, snapshot: CodexProviderBackup) -> Result<(), CodexBackendError> {
        for (path, previous) in snapshot.files {
            match previous {
                Some(content) => aam_core::atomic_write(&path, &content)?,
                None => {
                    if path.is_file() {
                        fs::remove_file(&path)?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileRegistry;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "aam-switcher-codex-test-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_profile_registers_and_creates_directory() {
        // `default_config_dir_for` is rooted at `aam_core::aam_home()`;
        // holding this lock for AAM_HOME's whole lifetime here prevents
        // racing with any other test in this crate that also points
        // AAM_HOME at its own throwaway directory.
        let _lock = crate::test_support::AAM_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let base = temp_dir("create-profile");
        std::env::set_var("AAM_HOME", &base);
        let registry = ProfileRegistry::open(base.join("profiles.json"));

        let profile = create_profile(&registry, "test-account").unwrap();

        assert!(profile.config_dir.is_dir());
        assert_eq!(registry.list().unwrap().len(), 1);

        std::env::remove_var("AAM_HOME");
        let _ = fs::remove_dir_all(&base);
    }

    /// A fake Provider whose `materialize`/`verify` are fully under test
    /// control, so the apply/rollback sequence can be exercised without a
    /// real Codex install or network access.
    struct FakeProvider {
        fail_verify: bool,
    }

    impl Provider for FakeProvider {
        fn id(&self) -> &str {
            "fake"
        }
        fn materialize(&self, target: ToolKind) -> ProviderConfig {
            match target {
                ToolKind::Codex { config_dir } => ProviderConfig {
                    codex_config_toml: Some("model_provider = \"fake\"\n".to_string()),
                    codex_token_file: Some(config_dir.join("provider-token.secret")),
                    codex_extra_files: vec![(config_dir.join("extra.json"), b"{}".to_vec())],
                    ..Default::default()
                },
                ToolKind::Claude => ProviderConfig::default(),
            }
        }
        fn verify(&self, _cfg: &ProviderConfig) -> Result<(), VerifyError> {
            if self.fail_verify {
                Err(VerifyError::UnexpectedResponse("forced test failure".into()))
            } else {
                Ok(())
            }
        }
        fn api_key(&self) -> &str {
            "fake-api-key"
        }
        fn complete(&self, _prompt: &str) -> Result<String, crate::provider::CompleteError> {
            Ok("fake completion".to_string())
        }
    }

    #[test]
    fn apply_provider_writes_config_and_extra_files() {
        let dir = temp_dir("apply-success");
        let provider = FakeProvider { fail_verify: false };
        let mut op = ApplyCodexProvider::new(dir.clone(), &provider);

        aam_core::execute(&mut op).expect("apply should succeed");

        assert!(dir.join("config.toml").is_file());
        assert!(dir.join("extra.json").is_file());
        assert!(dir.join("provider-token.secret").is_file());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_verify_rolls_back_to_previous_config_toml() {
        let dir = temp_dir("apply-rollback");
        fs::write(dir.join("config.toml"), b"# original config\n").unwrap();

        let provider = FakeProvider { fail_verify: true };
        let mut op = ApplyCodexProvider::new(dir.clone(), &provider);

        let result = aam_core::execute(&mut op);
        assert!(result.is_err(), "apply should fail because verify fails");

        let restored = fs::read_to_string(dir.join("config.toml")).unwrap();
        assert_eq!(restored, "# original config\n");
        // Files that didn't exist before must be cleaned up too.
        assert!(!dir.join("extra.json").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
