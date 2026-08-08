//! Session/Project Memory-Bank (`docs/05-session-memory-bank-module.md`,
//! Phase 3a): reads/writes the same `project-index.json`
//! `~/.claude/skills/project-tracker`'s hook already maintains (see
//! `index.rs`'s doc comment for why that's a deliberate departure from
//! every other `aam-*` registry living under `~/.aam`), and generalizes
//! its `backfill-index.ps1` into cross-tool, cross-Profile session
//! discovery (`scan.rs`) with an explicit scan-then-adopt-then-approve
//! flow (`adopt.rs`) so "discovered" and "synced" stay two separate,
//! user-controlled steps (`05.2`).
//!
//! WebDAV sync of this index (`aam session sync`) is not part of this
//! crate yet -- Phase 3a's scope is the local model and discovery/
//! adoption; wiring it to `aam-sync` lands in a follow-up round, the same
//! "engine, then domain object, then sync" pacing Phase 2 used.

mod adopt;
mod index;
mod record;
mod scan;

pub use adopt::{adopt_session, approve_all_scanned, approve_sync};
pub use index::{IndexError, ProjectIndex};
pub use record::ProjectRecord;
pub use scan::{scan_claude_sessions, scan_codex_sessions, DiscoveredSession};
