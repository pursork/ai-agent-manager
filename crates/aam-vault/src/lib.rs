//! Local credential vault — Phase 1 subset (see `docs/04-webdav-sync-security.md`
//! for the eventual WebDAV-synced version).
//!
//! Ships one thing: [`SecretStore`], an OS-appropriate encrypted-at-rest
//! key/value store for small secrets (Provider API keys, in Phase 1). This
//! deliberately does **not** try to be a generic vault abstraction yet —
//! account credentials themselves live directly in each Profile's
//! `CLAUDE_CONFIG_DIR`/`CODEX_HOME` directory (`aam-switcher`'s "N
//! directories" model, `docs/03-credential-account-module.md` §3.2), so
//! there is nothing else for this crate to store until Phase 2's WebDAV
//! sync work begins.
//!
//! Security model (`docs/02-architecture.md` §2.4, "本地态因 OS 而异是被
//! 接受的"): Windows uses DPAPI (`CurrentUser` scope), the same primitive
//! `codex-skill` already uses in production. Unix uses plaintext + `chmod
//! 600` — weaker, and documented as such rather than pretending otherwise.

mod store;

#[cfg(windows)]
mod windows_dpapi;

#[cfg(unix)]
mod unix_plain;

pub use store::{SecretStore, VaultError};
