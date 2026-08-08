// The DeepSeek model catalog's `serde_json::json!` literal (providers/deepseek.rs)
// is deeply nested enough to exceed the default macro recursion limit.
#![recursion_limit = "256"]

//! Account/Provider Switcher (`docs/03-credential-account-module.md`).
//!
//! Both Claude and Codex use the "N directories" model confirmed viable
//! by `docs/08-open-questions-risks.md` §8.1: switching means launching a
//! new process with `CLAUDE_CONFIG_DIR`/`CODEX_HOME` pointed at an
//! already-materialized Profile directory, never rewriting a live shared
//! config file in place.

mod claude;
mod codex;
mod codex_toml;
mod profile;
mod provider;
mod provider_registry;
mod provider_sync;
mod providers;
mod token_helper;
mod verify_http;

pub use claude::ClaudeBackendError;
pub use codex::{ApplyCodexProvider, CodexBackendError, CodexProviderBackup};
pub use profile::{default_config_dir_for, Profile, ProfileRegistry, RegistryError, Tool};
pub use provider::{Provider, ProviderConfig, ToolKind, VerifyError};
pub use provider_registry::{ProviderKind, ProviderRecord, ProviderRegistry, ProviderRegistryError};
pub use provider_sync::{
    blob_path_for as provider_blob_path_for, pull_provider, push_provider, reencrypt_all_known_providers,
    reencrypt_provider, ProviderSyncError,
};
pub use providers::{build_provider, catalog_file_name, CpaProvider, DeepSeekProvider};

/// Entropy label + key-namespace for Provider API keys in `aam-vault`
/// (distinct from `token_helper::TOKEN_ENTROPY`, which is per-Profile
/// copies materialized into a Codex Profile's `CODEX_HOME` -- this is the
/// one shared source of truth `aam profile use-provider` reads from).
pub const PROVIDER_SECRET_ENTROPY: &str = "aam-provider-secrets-v1";

pub fn provider_secret_store() -> std::io::Result<aam_vault::SecretStore> {
    aam_vault::SecretStore::new(aam_core::aam_home().join("provider-secrets"), PROVIDER_SECRET_ENTROPY)
}

/// The Claude backend's operations, grouped as functions rather than a
/// struct -- there's no persistent backend state beyond the Profile
/// registry, which callers already hold.
pub mod claude_backend {
    pub use crate::claude::{apply_provider, create_profile, launch_env, verify_login};
}

/// The Codex backend's operations (see [`claude_backend`] for why this is
/// functions-in-a-module rather than a struct).
pub mod codex_backend {
    pub use crate::codex::{create_profile, launch_env, verify_login};
}

/// Test-only support shared across this crate's `#[cfg(test)]` modules.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    /// Serializes any test that mutates the process-wide `AAM_HOME` env
    /// var (which `aam_core::aam_home()` reads on every call). `cargo
    /// test` runs a crate's tests in parallel threads within one process
    /// by default, so any two tests that both point `AAM_HOME` at their
    /// own throwaway directory race unless they hold this lock for their
    /// entire body.
    pub(crate) static AAM_HOME_ENV_LOCK: Mutex<()> = Mutex::new(());
}
