//! Shared types and utilities for the `ai-agent-manager` workspace.
//!
//! Phase 0 only ships the [`TransactionalOp`] pattern (see
//! `docs/02-architecture.md` §2.6) and the [`atomic_write`] helper it's
//! built on. `aam-vault`, `aam-switcher`, `aam-sync`, and `aam-memory` all
//! depend on this crate and are expected to implement `TransactionalOp` for
//! their own state-mutating operations (account switches, vault writes,
//! sync pushes, index updates) rather than rolling their own ad hoc
//! snapshot/rollback logic.

mod atomic;
mod home;
mod transactional;

pub use atomic::atomic_write;
pub use home::{aam_home, user_home_dir};
pub use transactional::{execute, ExecuteError, RollbackFailed, TransactionalOp};
