//! Cross-tool session discovery (`docs/05-session-memory-bank-module.md`
//! §5.7), generalizing `project-tracker`'s `backfill-index.ps1` -- which
//! only ever scanned one fixed `~/.claude` -- to any registered Profile's
//! directory, for both Claude and Codex.
//!
//! **Read-only**: nothing here writes to [`crate::ProjectIndex`] -- that's
//! `adopt.rs`'s job, by design (§5.7's "scan first, adopt second" two-
//! stage flow, symmetric with `09`'s Skills scan/adopt).
//!
//! Where `backfill-index.ps1` extracts `cwd` with a regex over raw text,
//! this parses each line as JSON and reads the `cwd` key directly --
//! verified against real transcript lines on this machine (`cwd` sits at
//! the top level of each JSONL line, alongside `sessionId`/`userType`),
//! and more robust than a hand-rolled regex substitute for the same
//! result.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One session found on disk that isn't in the index yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSession {
    pub path: String,
    pub name: String,
    pub last_session_id: String,
    /// Unix seconds. Kept as a plain integer rather than a formatted
    /// string here -- turning it into `ProjectRecord`'s opaque timestamp
    /// string is `adopt.rs`'s job, so this crate doesn't need a datetime-
    /// formatting dependency just for scan output.
    pub last_active_unix: i64,
    pub created_unix: i64,
    /// `05.8`: Claude sessions get this from their last `ai-title` record;
    /// Codex rollout files have no equivalent auto-summary field (verified
    /// against a real rollout file -- its `session_meta` header has no
    /// such field), so this is always `None` for `tool_kind == "codex"`.
    pub auto_status: Option<String>,
    pub tool_kind: &'static str,
    /// The winning transcript/rollout file this session's metadata came
    /// from (the latest one, if several shared the same `cwd`) --
    /// `adopt.rs`'s `--summarize` path reads a chunk of this file's raw
    /// content as the model's input; nothing else in this crate opens it.
    pub source_file: PathBuf,
}

fn unix_seconds(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn mtime_of(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Recursively finds every `*.jsonl` file under `root`, skipping any
/// directory whose name matches `exclude_dir_name` (e.g. Claude's
/// `subagents` transcripts, which aren't separate projects).
fn find_jsonl_files(root: &Path, exclude_dir_name: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let is_excluded = exclude_dir_name
                    .is_some_and(|excluded| path.file_name().and_then(|n| n.to_str()) == Some(excluded));
                if !is_excluded {
                    stack.push(path);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                out.push(path);
            }
        }
    }
    out
}

fn find_first_cwd(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    content.lines().find_map(|line| {
        serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|v| v.get("cwd").and_then(Value::as_str).map(str::to_string))
    })
}

fn find_latest_ai_title(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v.get("type").and_then(Value::as_str) == Some("ai-title"))
        .filter_map(|v| v.get("aiTitle").and_then(Value::as_str).map(str::to_string))
        .next_back()
}

fn project_name_from_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}

struct GroupEntry {
    cwd: String,
    latest_file: PathBuf,
    latest_mtime: SystemTime,
    earliest_mtime: SystemTime,
}

/// Groups files by `cwd` (case-insensitive), keeping the latest/earliest
/// mtime per group -- same dedup shape as `backfill-index.ps1`, so a
/// project with many session files becomes one discovered entry, not one
/// per file.
fn group_by_cwd(files: Vec<PathBuf>, exclude_prefixes: &[PathBuf]) -> HashMap<String, GroupEntry> {
    let excluded_lower: Vec<String> = exclude_prefixes
        .iter()
        .map(|p| p.to_string_lossy().to_lowercase())
        .collect();

    let mut by_cwd: HashMap<String, GroupEntry> = HashMap::new();
    for file in files {
        let Some(cwd) = find_first_cwd(&file) else {
            continue;
        };
        let cwd_lower = cwd.to_lowercase();
        if excluded_lower.iter().any(|prefix| cwd_lower.starts_with(prefix)) {
            continue;
        }
        let mtime = mtime_of(&file);
        by_cwd
            .entry(cwd_lower)
            .and_modify(|e| {
                if mtime > e.latest_mtime {
                    e.latest_file = file.clone();
                    e.latest_mtime = mtime;
                }
                if mtime < e.earliest_mtime {
                    e.earliest_mtime = mtime;
                }
            })
            .or_insert(GroupEntry {
                cwd,
                latest_file: file,
                latest_mtime: mtime,
                earliest_mtime: mtime,
            });
    }
    by_cwd
}

/// Discovers Claude Code sessions under `<profile_dir>/projects/**/*.jsonl`
/// not already present in `known_session_ids` (the `(path, lastSessionId)`
/// dedup key from `05.7` collapses here to just `lastSessionId`, since a
/// session id is already unique per file). `exclude_path_prefixes` is
/// typically `[profile_dir]` itself, to skip sessions run inside the
/// config directory -- same purpose as `backfill-index.ps1`'s
/// `$ExcludePrefixes`.
pub fn scan_claude_sessions(
    profile_dir: &Path,
    exclude_path_prefixes: &[PathBuf],
    known_session_ids: &[String],
) -> Vec<DiscoveredSession> {
    let projects_root = profile_dir.join("projects");
    if !projects_root.is_dir() {
        return Vec::new();
    }

    let files = find_jsonl_files(&projects_root, Some("subagents"));
    let groups = group_by_cwd(files, exclude_path_prefixes);

    groups
        .into_values()
        .filter_map(|entry| {
            let session_id = entry
                .latest_file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if known_session_ids.contains(&session_id) {
                return None;
            }
            Some(DiscoveredSession {
                name: project_name_from_path(&entry.cwd),
                path: entry.cwd,
                last_session_id: session_id,
                last_active_unix: unix_seconds(entry.latest_mtime),
                created_unix: unix_seconds(entry.earliest_mtime),
                auto_status: find_latest_ai_title(&entry.latest_file),
                tool_kind: "claude",
                source_file: entry.latest_file,
            })
        })
        .collect()
}

/// Discovers Codex sessions under `<profile_dir>/sessions/**/rollout-*.jsonl`.
/// Reads each file's first line (`session_meta` event) for `payload.cwd`/
/// `payload.id` -- confirmed against a real rollout file on this machine,
/// not assumed. No `session_index.jsonl` fast-path: confirmed absent on
/// this machine (`docs/08-open-questions-risks.md` #10), so this scans
/// `sessions/**` directly as the primary (not fallback) mechanism.
pub fn scan_codex_sessions(profile_dir: &Path, known_session_ids: &[String]) -> Vec<DiscoveredSession> {
    let sessions_root = profile_dir.join("sessions");
    if !sessions_root.is_dir() {
        return Vec::new();
    }

    let files: Vec<PathBuf> = find_jsonl_files(&sessions_root, None)
        .into_iter()
        .filter(|f| {
            f.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("rollout-"))
        })
        .collect();

    // `group_by_cwd`/`find_first_cwd` assume `cwd` sits at a JSONL line's
    // top level (true for Claude's transcripts, confirmed on this
    // machine) -- Codex's rollout format nests it under `payload`
    // instead, so this does its own grouping rather than reusing that
    // Claude-shaped helper.
    struct CodexEntry {
        cwd: String,
        session_id: String,
        latest_file: PathBuf,
        latest_mtime: SystemTime,
        earliest_mtime: SystemTime,
    }
    let mut by_cwd: HashMap<String, CodexEntry> = HashMap::new();

    for file in &files {
        let Ok(content) = fs::read_to_string(file) else {
            continue;
        };
        let Some(first_line) = content.lines().next() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(first_line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        let (Some(cwd), Some(session_id)) = (
            payload.get("cwd").and_then(Value::as_str),
            payload.get("id").and_then(Value::as_str),
        ) else {
            continue;
        };

        let mtime = mtime_of(file);
        let key = cwd.to_lowercase();
        by_cwd
            .entry(key)
            .and_modify(|e| {
                if mtime > e.latest_mtime {
                    e.latest_file = file.clone();
                    e.latest_mtime = mtime;
                    e.session_id = session_id.to_string();
                }
                if mtime < e.earliest_mtime {
                    e.earliest_mtime = mtime;
                }
            })
            .or_insert(CodexEntry {
                cwd: cwd.to_string(),
                session_id: session_id.to_string(),
                latest_file: file.clone(),
                latest_mtime: mtime,
                earliest_mtime: mtime,
            });
    }

    by_cwd
        .into_values()
        .filter(|entry| !known_session_ids.contains(&entry.session_id))
        .map(|entry| DiscoveredSession {
            name: project_name_from_path(&entry.cwd),
            path: entry.cwd,
            last_session_id: entry.session_id,
            last_active_unix: unix_seconds(entry.latest_mtime),
            created_unix: unix_seconds(entry.earliest_mtime),
            auto_status: None,
            tool_kind: "codex",
            source_file: entry.latest_file,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "aam-memory-scan-test-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_claude_transcript(path: &Path, cwd: &str, session_id: &str, ai_title: Option<&str>) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut lines = vec![format!(
            r#"{{"type":"user","cwd":"{}","sessionId":"{}"}}"#,
            cwd.replace('\\', "\\\\"),
            session_id
        )];
        if let Some(title) = ai_title {
            lines.push(format!(
                r#"{{"type":"ai-title","aiTitle":"{title}","sessionId":"{session_id}"}}"#
            ));
        }
        fs::write(path, lines.join("\n")).unwrap();
    }

    fn write_codex_rollout(path: &Path, cwd: &str, session_id: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let line = format!(
            r#"{{"timestamp":"2026-03-27T02:44:40.202Z","type":"session_meta","payload":{{"id":"{session_id}","timestamp":"2026-03-27T02:44:40.056Z","cwd":"{}","originator":"codex_cli_rs","cli_version":"0.42.0"}}}}"#,
            cwd.replace('\\', "\\\\")
        );
        fs::write(path, line).unwrap();
    }

    #[test]
    fn scan_claude_sessions_discovers_a_new_project() {
        let dir = TempDir::new("claude-basic");
        let transcript = dir
            .0
            .join("projects")
            .join("some-key")
            .join("session-a.jsonl");
        write_claude_transcript(&transcript, "D:\\research\\Gear-Sys", "session-a", Some("修好了齿轮问题"));

        let found = scan_claude_sessions(&dir.0, &[], &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "D:\\research\\Gear-Sys");
        assert_eq!(found[0].name, "Gear-Sys");
        assert_eq!(found[0].last_session_id, "session-a");
        assert_eq!(found[0].auto_status.as_deref(), Some("修好了齿轮问题"));
        assert_eq!(found[0].tool_kind, "claude");
    }

    #[test]
    fn scan_claude_sessions_skips_subagent_transcripts() {
        let dir = TempDir::new("claude-subagents");
        let transcript = dir
            .0
            .join("projects")
            .join("some-key")
            .join("session-a")
            .join("subagents")
            .join("sub-1.jsonl");
        write_claude_transcript(&transcript, "D:\\research\\Gear-Sys", "sub-1", None);

        let found = scan_claude_sessions(&dir.0, &[], &[]);
        assert!(found.is_empty());
    }

    #[test]
    fn scan_claude_sessions_excludes_configured_prefixes() {
        let dir = TempDir::new("claude-exclude");
        let transcript = dir.0.join("projects").join("key").join("session-a.jsonl");
        // cwd is inside the profile dir itself -- should be excluded,
        // same as project-tracker's own $HOME/.claude exclusion.
        let cwd = dir.0.to_string_lossy().to_string();
        write_claude_transcript(&transcript, &cwd, "session-a", None);

        let found = scan_claude_sessions(&dir.0, std::slice::from_ref(&dir.0), &[]);
        assert!(found.is_empty());
    }

    #[test]
    fn scan_claude_sessions_dedups_already_known_session_ids() {
        let dir = TempDir::new("claude-known");
        let transcript = dir.0.join("projects").join("key").join("session-a.jsonl");
        write_claude_transcript(&transcript, "D:\\research\\Gear-Sys", "session-a", None);

        let found = scan_claude_sessions(&dir.0, &[], &["session-a".to_string()]);
        assert!(found.is_empty());
    }

    #[test]
    fn scan_claude_sessions_groups_multiple_files_by_cwd_keeping_latest() {
        let dir = TempDir::new("claude-group");
        let older = dir.0.join("projects").join("key").join("session-old.jsonl");
        let newer = dir.0.join("projects").join("key").join("session-new.jsonl");
        write_claude_transcript(&older, "D:\\research\\Gear-Sys", "session-old", Some("old status"));
        write_claude_transcript(&newer, "D:\\research\\Gear-Sys", "session-new", Some("new status"));

        // Force distinguishable mtimes (filesystem write order alone isn't
        // guaranteed to be monotonic within the same test run's precision).
        let now = SystemTime::now();
        filetime_set(&older, now - std::time::Duration::from_secs(120));
        filetime_set(&newer, now);

        let found = scan_claude_sessions(&dir.0, &[], &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].last_session_id, "session-new");
        assert_eq!(found[0].auto_status.as_deref(), Some("new status"));
    }

    #[test]
    fn scan_codex_sessions_discovers_a_new_project() {
        let dir = TempDir::new("codex-basic");
        let rollout = dir
            .0
            .join("sessions")
            .join("2026")
            .join("03")
            .join("27")
            .join("rollout-2026-03-27T02-44-40-session-a.jsonl");
        write_codex_rollout(&rollout, "C:\\Users\\x\\project", "session-a");

        let found = scan_codex_sessions(&dir.0, &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "C:\\Users\\x\\project");
        assert_eq!(found[0].last_session_id, "session-a");
        assert_eq!(found[0].auto_status, None);
        assert_eq!(found[0].tool_kind, "codex");
    }

    #[test]
    fn scan_codex_sessions_dedups_already_known_session_ids() {
        let dir = TempDir::new("codex-known");
        let rollout = dir
            .0
            .join("sessions")
            .join("2026")
            .join("03")
            .join("27")
            .join("rollout-x.jsonl");
        write_codex_rollout(&rollout, "C:\\Users\\x\\project", "session-a");

        let found = scan_codex_sessions(&dir.0, &["session-a".to_string()]);
        assert!(found.is_empty());
    }

    #[test]
    fn scan_ignores_non_jsonl_and_malformed_files() {
        let dir = TempDir::new("malformed");
        fs::create_dir_all(dir.0.join("projects").join("key")).unwrap();
        fs::write(dir.0.join("projects").join("key").join("not-json.txt"), "hello").unwrap();
        fs::write(
            dir.0.join("projects").join("key").join("broken.jsonl"),
            "{not valid json",
        )
        .unwrap();

        let found = scan_claude_sessions(&dir.0, &[], &[]);
        assert!(found.is_empty());
    }

    /// Minimal mtime setter via `filetime`-free means: reopen and touch via
    /// `std::fs::File::set_modified`, available on the stable std since
    /// 1.75 -- avoids adding the `filetime` crate for one test helper.
    fn filetime_set(path: &Path, time: SystemTime) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(time).unwrap();
    }
}
