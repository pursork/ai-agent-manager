//! Placeholder crate — Phase 1 (see `docs/03-credential-account-module.md`)
//! will turn this into the Account/Provider switcher: the Claude
//! (`CLAUDE_CONFIG_DIR` directory-selection) and Codex backends, the
//! `Provider` trait for third-party endpoints (CPA, DeepSeek V4 Flash),
//! and the standard snapshot → apply → verify → rollback switch sequence
//! built on `aam_core::TransactionalOp`.
//!
//! Phase 0 only exists to prove out the workspace's crate graph and to give
//! `aam-cli` a real (if empty) dependency to build against.

/// Phase 0 placeholder confirming `aam-switcher` compiles as part of the
/// workspace. Superseded once Phase 1 adds real switching logic.
pub fn placeholder() -> &'static str {
    "aam-switcher (Phase 0 placeholder, depends on aam-vault + aam-sync)"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_mentions_crate_name() {
        assert!(placeholder().contains("aam-switcher"));
    }

    /// Exercises the `aam-vault` and `aam-sync` dependency edges per
    /// `docs/02-architecture.md` §2.1's dependency direction
    /// (`aam-switcher` → `aam-vault` + `aam-sync`).
    #[test]
    fn depends_on_aam_vault_and_aam_sync() {
        assert!(aam_vault::placeholder().contains("aam-vault"));
        assert!(aam_sync::placeholder().contains("aam-sync"));
    }
}
