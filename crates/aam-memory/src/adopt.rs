//! `docs/05-session-memory-bank-module.md` §5.8: writes
//! [`DiscoveredSession`]s found by `scan.rs` into the local
//! [`ProjectIndex`], with `discoverySource: "scan"` and
//! `syncApproved: false`.
//!
//! `--summarize`'s actual Provider call (generating `autoStatus` for a
//! Codex session, which has no built-in equivalent to Claude's
//! `ai-title`) lives in `aam-cli`, not here -- `aam-memory` can't depend
//! on `aam-switcher` (same layering `provider_sync`/`account_sync` follow
//! in the other direction). This module accepts an already-computed
//! summary string as a parameter instead of calling out to a Provider
//! itself.

use crate::index::{IndexError, ProjectIndex};
use crate::record::ProjectRecord;
use crate::scan::DiscoveredSession;

fn to_timestamp_string(unix_seconds: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(unix_seconds)
        .ok()
        .and_then(|t| t.format(&time::format_description::well_known::Rfc3339).ok())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

/// Writes `session` into `index` as a new record. `device_id` is this
/// machine's identity (reuses whatever the caller already has -- e.g.
/// `aam_sync::local_identity`'s `device_id`, if a vault has been set up;
/// an empty string is fine too, same as pre-Phase-3 records that predate
/// this field). `profile_label` is the Profile the scan was run against.
/// `summary_override`, if given, becomes `autoStatus` (the `--summarize`
/// path); otherwise `session.auto_status` (Claude's own `ai-title`, or
/// `None` for Codex) is used as-is.
pub fn adopt_session(
    index: &ProjectIndex,
    session: &DiscoveredSession,
    device_id: &str,
    profile_label: &str,
    summary_override: Option<String>,
) -> Result<(), IndexError> {
    let record = ProjectRecord {
        path: session.path.clone(),
        name: session.name.clone(),
        last_session_id: session.last_session_id.clone(),
        last_active: to_timestamp_string(session.last_active_unix),
        created: to_timestamp_string(session.created_unix),
        auto_status: summary_override.or_else(|| session.auto_status.clone()),
        status_override: None,
        auth_backend: None,
        device_id: device_id.to_string(),
        tool_kind: session.tool_kind.to_string(),
        profile_label: Some(profile_label.to_string()),
        full_sync_enabled: false,
        full_sync_status: None,
        discovery_source: "scan".to_string(),
        sync_approved: false,
        project_id: None,
    };
    index.upsert(record)
}

/// Marks the records at `paths` (case-insensitive match against
/// `ProjectRecord::path`, same key `ProjectIndex` uses everywhere) as
/// approved for sync (`05.9`). Records not found in the index are simply
/// skipped -- the caller (`aam-cli`) is responsible for telling the user
/// which names didn't match anything, this function just applies what it
/// can.
pub fn approve_sync(index: &ProjectIndex, paths: &[String]) -> Result<usize, IndexError> {
    let mut approved = 0;
    for path in paths {
        if index.update(path, |r| r.sync_approved = true).is_ok() {
            approved += 1;
        }
    }
    Ok(approved)
}

/// `aam session approve-sync --all-scanned`: approves every record whose
/// `discoverySource == "scan"` and isn't already approved.
pub fn approve_all_scanned(index: &ProjectIndex) -> Result<usize, IndexError> {
    let targets: Vec<String> = index
        .list()?
        .into_iter()
        .filter(|r| r.discovery_source == "scan" && !r.sync_approved)
        .map(|r| r.path)
        .collect();
    approve_sync(index, &targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_index(label: &str) -> (ProjectIndex, PathBuf) {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "aam-memory-adopt-test-{label}-{}-{unique}",
            std::process::id()
        ));
        (ProjectIndex::open(dir.join("project-index.json")), dir)
    }

    fn sample_session() -> DiscoveredSession {
        DiscoveredSession {
            path: "D:\\research\\Gear-Sys".into(),
            name: "Gear-Sys".into(),
            last_session_id: "session-a".into(),
            last_active_unix: 1_770_000_000,
            created_unix: 1_760_000_000,
            auto_status: Some("修好了齿轮问题".into()),
            tool_kind: "claude",
            source_file: "D:\\research\\Gear-Sys\\session-a.jsonl".into(),
        }
    }

    #[test]
    fn adopt_session_writes_a_scan_sourced_unapproved_record() {
        let (index, dir) = temp_index("adopt-basic");
        adopt_session(&index, &sample_session(), "device-1", "官方账号1", None).unwrap();

        let record = index.get("D:\\research\\Gear-Sys").unwrap().unwrap();
        assert_eq!(record.discovery_source, "scan");
        assert!(!record.sync_approved);
        assert_eq!(record.device_id, "device-1");
        assert_eq!(record.profile_label.as_deref(), Some("官方账号1"));
        assert_eq!(record.auto_status.as_deref(), Some("修好了齿轮问题"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn adopt_session_summary_override_takes_precedence() {
        let (index, dir) = temp_index("adopt-summary");
        adopt_session(
            &index,
            &sample_session(),
            "device-1",
            "官方账号1",
            Some("AI 生成的摘要".to_string()),
        )
        .unwrap();

        let record = index.get("D:\\research\\Gear-Sys").unwrap().unwrap();
        assert_eq!(record.auto_status.as_deref(), Some("AI 生成的摘要"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn approve_sync_flips_only_named_records() {
        let (index, dir) = temp_index("approve-named");
        let mut session_b = sample_session();
        session_b.path = "D:\\research\\Widget".into();
        adopt_session(&index, &sample_session(), "d", "p", None).unwrap();
        adopt_session(&index, &session_b, "d", "p", None).unwrap();

        let approved = approve_sync(&index, &["D:\\research\\Gear-Sys".to_string()]).unwrap();
        assert_eq!(approved, 1);

        assert!(index.get("D:\\research\\Gear-Sys").unwrap().unwrap().sync_approved);
        assert!(!index.get("D:\\research\\Widget").unwrap().unwrap().sync_approved);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn approve_all_scanned_ignores_live_records() {
        let (index, dir) = temp_index("approve-all-scanned");
        adopt_session(&index, &sample_session(), "d", "p", None).unwrap();

        let mut live_record = index.get("D:\\research\\Gear-Sys").unwrap().unwrap();
        live_record.path = "D:\\research\\LiveOne".into();
        live_record.discovery_source = "live".into();
        live_record.sync_approved = true;
        index.upsert(live_record).unwrap();

        let approved = approve_all_scanned(&index).unwrap();
        assert_eq!(approved, 1);
        assert!(index.get("D:\\research\\Gear-Sys").unwrap().unwrap().sync_approved);
        let _ = fs::remove_dir_all(dir);
    }
}
