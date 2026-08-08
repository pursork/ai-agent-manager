//! Placeholder crate — Phase 3 (see `docs/05-session-memory-bank-module.md`)
//! will turn this into the Session/Project Memory-Bank tracker: the local
//! `project-index.json`-compatible cache (extending `project-tracker`'s
//! schema) plus the WebDAV-synced cross-device index.
//!
//! Phase 0 only exists to prove out the workspace's crate graph and to give
//! `aam-cli` a real (if empty) dependency to build against.

/// Phase 0 placeholder confirming `aam-memory` compiles as part of the
/// workspace. Superseded once Phase 3 adds real project/session tracking.
pub fn placeholder() -> &'static str {
    "aam-memory (Phase 0 placeholder, depends on aam-vault + aam-sync)"
}

#[cfg(test)]
mod tests {
    use super::*;
    use aam_sync::SyncBackend;

    #[test]
    fn placeholder_mentions_crate_name() {
        assert!(placeholder().contains("aam-memory"));
    }

    /// Exercises the `aam-vault` and `aam-sync` dependency edges per
    /// `docs/02-architecture.md` §2.1's dependency direction
    /// (`aam-memory` → `aam-vault` + `aam-sync`). `aam-sync` stopped being
    /// a Phase 0 placeholder in Phase 2 (`docs/04-webdav-sync-security.md`),
    /// so this now exercises its real `SyncBackend` API instead of a
    /// `placeholder()` stub.
    #[test]
    fn depends_on_aam_vault_and_aam_sync() {
        let dir = std::env::temp_dir().join(format!("aam-memory-dep-check-{}", std::process::id()));
        let store = aam_vault::SecretStore::new(&dir, "aam-memory-dep-check-v1").unwrap();
        store.save("k", "v").unwrap();
        assert_eq!(store.load("k").unwrap().as_deref(), Some("v"));
        let _ = std::fs::remove_dir_all(&dir);

        let sync_dir = std::env::temp_dir().join(format!("aam-memory-sync-dep-check-{}", std::process::id()));
        let backend = aam_sync::LocalDirBackend::new(&sync_dir);
        backend.put("p", b"v").unwrap();
        assert_eq!(backend.get("p").unwrap(), Some(b"v".to_vec()));
        let _ = std::fs::remove_dir_all(&sync_dir);
    }
}
