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
//! Phase 3b adds WebDAV sync of this index (`sync.rs`, `aam session
//! sync`) -- see that module's doc comment for why the shared blob needs
//! pull-merge-push semantics instead of the simple per-id
//! `push_if_not_stale` `provider_sync`/`account_sync` get away with, and
//! why pulled cross-device records are mirrored into a separate aam-owned
//! file rather than written into `project-tracker`'s own.

mod adopt;
mod index;
mod project_link;
mod record;
mod scan;
mod sync;

pub use adopt::{adopt_session, approve_all_scanned, approve_sync};
pub use index::{IndexError, ProjectIndex};
pub use project_link::{link_projects, LinkError};
pub use record::ProjectRecord;
pub use scan::{scan_claude_sessions, scan_codex_sessions, DiscoveredSession};
pub use sync::{remote_mirror_index, remote_mirror_path, sync_index, MemorySyncError};
