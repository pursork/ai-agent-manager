//! Placeholder crate — Phase 2 (see `docs/04-webdav-sync-security.md`) will
//! turn this into the local credential vault: encrypted account/provider
//! secrets, device identity (age keypair), and primary-password unlock.
//!
//! Phase 0 only exists to prove out the workspace's crate graph and to give
//! `aam-switcher`/`aam-memory` a real (if empty) dependency to build against.

/// Phase 0 placeholder confirming `aam-vault` compiles as part of the
/// workspace. Superseded once Phase 2 adds real vault operations.
pub fn placeholder() -> &'static str {
    "aam-vault (Phase 0 placeholder, depends on aam-core)"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    #[test]
    fn placeholder_mentions_crate_name() {
        assert!(placeholder().contains("aam-vault"));
    }

    /// Exercises the `aam-core` dependency edge (not just declares it) by
    /// implementing `TransactionalOp` with it, per Phase 0's requirement
    /// that the workspace's declared dependency graph is actually compiled
    /// and linked, not merely listed in `Cargo.toml`.
    #[test]
    fn depends_on_aam_core() {
        struct NoopOp;
        impl aam_core::TransactionalOp for NoopOp {
            type Snapshot = ();
            type Error = Infallible;
            fn snapshot(&self) -> Result<(), Infallible> {
                Ok(())
            }
            fn apply(&mut self) -> Result<(), Infallible> {
                Ok(())
            }
            fn verify(&self) -> Result<(), Infallible> {
                Ok(())
            }
            fn rollback(&mut self, _snapshot: ()) -> Result<(), Infallible> {
                Ok(())
            }
        }

        let mut op = NoopOp;
        aam_core::execute(&mut op).expect("noop transactional op should succeed");
    }
}
