//! Claude backend: the "N directories" account model (`CLAUDE_CONFIG_DIR`
//! selection at launch, `docs/03` §3.2), plus Skills consistency
//! provisioning (`03.7`) at Profile-creation time.
//!
//! Applying a Provider to a Claude Profile is much simpler than Codex's
//! (`codex.rs`): Phase 1 Claude Providers materialize to environment
//! variables only (`03.4`'s "方案 B" -- directly rewriting
//! `ANTHROPIC_BASE_URL`/`ANTHROPIC_API_KEY`), injected fresh at each
//! launch. There is no persisted file to snapshot/apply/rollback --
//! "applying" a Provider is just recording the association in the
//! Profile registry, after a real (network) verify.

use crate::profile::{default_config_dir_for, Profile, ProfileRegistry, RegistryError, Tool};
use crate::provider::{Provider, ToolKind, VerifyError};
use std::error::Error;
use std::fmt;
use std::io;

#[derive(Debug)]
pub enum ClaudeBackendError {
    Registry(RegistryError),
    Io(io::Error),
    Skills(aam_core::ExecuteError<aam_skills::LinkError>),
    Verify(VerifyError),
}

impl fmt::Display for ClaudeBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClaudeBackendError::Registry(e) => write!(f, "{e}"),
            ClaudeBackendError::Io(e) => write!(f, "I/O error: {e}"),
            ClaudeBackendError::Skills(e) => write!(f, "skills provisioning failed: {e}"),
            ClaudeBackendError::Verify(e) => write!(f, "{e}"),
        }
    }
}

impl Error for ClaudeBackendError {}

impl From<RegistryError> for ClaudeBackendError {
    fn from(e: RegistryError) -> Self {
        ClaudeBackendError::Registry(e)
    }
}
impl From<io::Error> for ClaudeBackendError {
    fn from(e: io::Error) -> Self {
        ClaudeBackendError::Io(e)
    }
}

/// Creates a new Claude Profile: a fresh `CLAUDE_CONFIG_DIR`-equivalent
/// directory with its `skills/` subpath linked to the canonical
/// `~/.claude/skills` store (`03.7`), registered in `registry`. Actually
/// logging in (`claude auth login`) is left to the user via
/// `aam claude <label>` -- see `docs/03` §3.6.
pub fn create_profile(registry: &ProfileRegistry, label: &str) -> Result<Profile, ClaudeBackendError> {
    let config_dir = default_config_dir_for(Tool::Claude, label);
    std::fs::create_dir_all(&config_dir)?;

    aam_skills::provision_profile_skills_link(&config_dir).map_err(ClaudeBackendError::Skills)?;

    let profile = Profile {
        label: label.to_string(),
        tool: Tool::Claude,
        config_dir,
        provider: None,
    };
    registry.add(profile.clone())?;
    Ok(profile)
}

/// Verifies `provider` actually works, then records the association on
/// `profile` in `registry`. Never persists an association that didn't
/// verify.
pub fn apply_provider(
    registry: &ProfileRegistry,
    profile: &Profile,
    provider: &dyn Provider,
) -> Result<(), ClaudeBackendError> {
    let cfg = provider.materialize(ToolKind::Claude);
    provider.verify(&cfg).map_err(ClaudeBackendError::Verify)?;
    registry.set_provider(Tool::Claude, &profile.label, Some(provider.id().to_string()))?;
    Ok(())
}

/// Environment variables to set when launching `claude` under this
/// Profile, optionally with `provider`'s env vars layered on top.
pub fn launch_env(profile: &Profile, provider: Option<&dyn Provider>) -> Vec<(String, String)> {
    let mut env = vec![(
        Tool::Claude.config_dir_env_var().to_string(),
        profile.config_dir.display().to_string(),
    )];
    if let Some(provider) = provider {
        env.extend(provider.materialize(ToolKind::Claude).env_vars);
    }
    env
}

/// Runs `claude auth status` under this Profile's `CLAUDE_CONFIG_DIR`.
/// Exit code 0 = logged in, 1 = not logged in (`docs/08-open-questions-risks.md`
/// §8.1, confirmed official command, fixed in Claude Code 2.1.41).
pub fn verify_login(profile: &Profile) -> io::Result<bool> {
    let output = std::process::Command::new("claude")
        .args(["auth", "status"])
        .env(Tool::Claude.config_dir_env_var(), &profile.config_dir)
        .output()?;
    Ok(output.status.success())
}

// No unit tests here: `create_profile` unavoidably provisions a link
// against the *real* `~/.claude/skills` (aam-skills has no override for
// that path -- unlike `AAM_HOME`, it must always track the real user
// home directory, `paths.rs`), so exercising it isn't hermetic enough
// for an automated test on a developer's real machine. The link
// mechanism itself is fully covered by `aam-skills`' own tests against
// temp directories; this module's plumbing is exercised end-to-end via
// `aam profile add --tool claude <label>` (see docs/07-roadmap.md Phase 1
// verification).
