//! Shared "how do we launch this Profile" logic -- used by both the
//! Profiles screen (open a terminal for day-to-day use / interactive
//! login) and the Projects screen (open a terminal to resume a specific
//! session). Extracted out of `screens/profiles.rs` in Phase 4 Round 2
//! so a second consumer doesn't have to duplicate it (see the Round 2
//! plan's design item 1).

use aam_switcher::{claude_backend, codex_backend, Profile, Provider, ProviderRecord, Tool};

/// Looks up the `Provider` a Profile has attached (if any) and loads its
/// API key from `aam-vault`, ready to hand to `claude_backend::launch_env`.
/// `None` covers both "no Provider attached" (official subscription) and
/// "attached but the record/key is missing" -- callers treat both the
/// same way (fall back to no Provider env vars) since a half-broken
/// Provider reference shouldn't block launching under the official
/// subscription's defaults.
pub fn resolve_provider(profile: &Profile, providers: &[ProviderRecord]) -> Option<Box<dyn Provider>> {
    let id = profile.provider.as_ref()?;
    let record = providers.iter().find(|p| &p.id == id)?.clone();
    let key = aam_switcher::provider_secret_store().ok()?.load(&record.id).ok()??;
    Some(aam_switcher::build_provider(&record, key))
}

/// Environment variables to launch `tool` under `profile`, with its
/// attached Provider's env vars layered on top if it has one and Claude
/// is the tool (Codex's Provider env vars are baked into `config.toml` by
/// `ApplyCodexProvider` at assignment time, not injected at launch --
/// `codex_backend::launch_env` reflects that; see `codex.rs`).
pub fn launch_env(tool: Tool, profile: &Profile, providers: &[ProviderRecord]) -> Vec<(String, String)> {
    match tool {
        Tool::Claude => {
            let provider_obj = resolve_provider(profile, providers);
            claude_backend::launch_env(profile, provider_obj.as_deref())
        }
        Tool::Codex => codex_backend::launch_env(profile),
    }
}
