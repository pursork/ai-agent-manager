//! The `Provider` trait (`docs/03-credential-account-module.md` §3.4):
//! third-party/self-hosted endpoints (CPA, DeepSeek V4 Flash in Phase 1)
//! that a Profile can be paired with instead of the official subscription.

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

/// Which tool a Provider is being materialized for, carrying whatever
/// context that tool's materialization actually needs. Codex's
/// command-backed bearer token mechanism needs to embed absolute paths
/// (to the helper script and the encrypted token file) inside the
/// generated `config.toml`, so its variant carries the Profile's
/// `CODEX_HOME` directory; Claude's Phase 1 mechanism (env vars only,
/// `03.4`) needs no extra context.
#[derive(Debug, Clone)]
pub enum ToolKind {
    Claude,
    Codex { config_dir: PathBuf },
}

/// What a Provider needs injected/written for a specific tool, produced
/// by [`Provider::materialize`]. Applying it (actually writing files,
/// snapshotting, rolling back on failure) is the backend's job
/// (`aam-switcher`'s `claude`/`codex` modules), not the Provider's --
/// `materialize` is a pure data-producing step.
#[derive(Debug, Clone, Default)]
pub struct ProviderConfig {
    /// Environment variables to inject when launching the tool's process.
    /// This is the entirety of Claude's Phase 1 mechanism (`ANTHROPIC_BASE_URL`/
    /// `ANTHROPIC_API_KEY`, `03.4`'s "方案 B").
    pub env_vars: Vec<(String, String)>,
    /// Codex only: the complete `model_provider`/`model_providers.<id>`/
    /// `[model_providers.<id>.auth]` TOML block to write into config.toml.
    pub codex_config_toml: Option<String>,
    /// Codex only: the command-backed bearer token helper script content
    /// that `codex_config_toml`'s `auth.command`/`args` reference.
    pub codex_token_helper_script: Option<String>,
    /// Codex only: absolute path the helper script expects its
    /// `-TokenFile` argument to point at (the backend writes the
    /// encrypted API key there).
    pub codex_token_file: Option<PathBuf>,
    /// Codex only: any additional sidecar files a Provider needs written
    /// (e.g. DeepSeek's `model_catalog_json` file) as `(absolute path,
    /// content)` pairs -- generic rather than a named field, so adding a
    /// new Provider that needs its own extra file never requires changing
    /// this struct (`docs/03` §3.4's extensibility goal).
    pub codex_extra_files: Vec<(PathBuf, Vec<u8>)>,
}

#[derive(Debug)]
pub enum VerifyError {
    /// The HTTP request itself failed (network, TLS, timeout, ...).
    Http(String),
    /// Got a response, but it didn't look like what a working Provider
    /// should return (non-2xx, unparseable JSON, expected model id
    /// missing from `data[]`, ...).
    UnexpectedResponse(String),
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerifyError::Http(msg) => write!(f, "request failed: {msg}"),
            VerifyError::UnexpectedResponse(msg) => write!(f, "unexpected response: {msg}"),
        }
    }
}

impl Error for VerifyError {}

#[derive(Debug)]
pub enum CompleteError {
    /// The HTTP request itself failed (network, TLS, timeout, ...).
    Http(String),
    /// Got a response, but it didn't look like a valid Anthropic Messages
    /// API reply (non-2xx, unparseable JSON, no `text`-typed content
    /// block, ...).
    UnexpectedResponse(String),
}

impl fmt::Display for CompleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompleteError::Http(msg) => write!(f, "request failed: {msg}"),
            CompleteError::UnexpectedResponse(msg) => write!(f, "unexpected response: {msg}"),
        }
    }
}

impl Error for CompleteError {}

/// A third-party/self-hosted Provider a Profile can be paired with.
///
/// `codex-skill`'s "account 与 provider 严格隔离成两个状态机" boundary
/// applies here too: a `Provider` impl only ever produces
/// tool-configuration content and checks its own reachability -- it never
/// touches account credentials (`auth.json`/Claude's OAuth state).
pub trait Provider {
    /// Stable identifier, e.g. `"cpa"` / `"deepseek-v4-flash"`.
    fn id(&self) -> &str;

    /// Generates the content this Provider needs written/injected for
    /// `target`. Pure: no filesystem/network side effects.
    fn materialize(&self, target: ToolKind) -> ProviderConfig;

    /// Confirms the Provider is actually reachable and serving the
    /// configured model right now -- not just that `materialize`'s output
    /// is well-formed. Phase 1 implements this as `GET {base_url}/models`
    /// (mirrors `codex-skill`'s `Test-ModelsEndpoint`); the deeper
    /// `Test-ResponsesEndpoint`/real-`codex exec` checks `codex-skill`
    /// also has are a later enhancement, not required for a Profile to be
    /// usable.
    fn verify(&self, cfg: &ProviderConfig) -> Result<(), VerifyError>;

    /// The plaintext secret this Provider authenticates with.
    /// `materialize`'s Codex output only contains the *path* the token
    /// should end up at (`ProviderConfig::codex_token_file`), not the
    /// token itself -- the Codex backend needs this to actually populate
    /// that file via `aam-vault` (`codex.rs`'s `ApplyCodexProvider`).
    fn api_key(&self) -> &str;

    /// Sends `prompt` as a single user message, returns the model's text
    /// reply. Uses the Anthropic Messages API (`POST {base_url}/v1/messages`,
    /// `X-Api-Key` auth) -- verified against Anthropic's own docs, not
    /// assumed, and consistent with what `materialize()` already commits
    /// these Providers to supporting for Claude Code to work at all
    /// (`docs/08-open-questions-risks.md` #17). Used by `aam session
    /// adopt --summarize` (`05.8`); has no relationship to the official
    /// subscription (no `Provider` exists for that -- `03.1`).
    fn complete(&self, prompt: &str) -> Result<String, CompleteError>;
}
