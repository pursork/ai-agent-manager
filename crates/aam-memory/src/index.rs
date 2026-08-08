//! `ProjectIndex`: reads/writes `project-index.json`.
//!
//! **Deliberately not rooted at `aam_core::aam_home()`** like every other
//! `aam-*` registry so far -- `docs/08-open-questions-risks.md` #9's
//! bridging decision is to read/write the *same file*
//! `~/.claude/skills/project-tracker`'s hook already maintains
//! (`$HOME/.claude/project-index.json`), not fork a duplicate under
//! `~/.aam`. `open_default()` reflects that; `open(path)` exists for tests
//! so they never touch the developer's real project index.

use crate::record::ProjectRecord;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct IndexFile {
    #[serde(default)]
    projects: Vec<ProjectRecord>,
}

#[derive(Debug)]
pub enum IndexError {
    Io(io::Error),
    Json(serde_json::Error),
    NotFound(String),
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IndexError::Io(e) => write!(f, "project index I/O error: {e}"),
            IndexError::Json(e) => write!(f, "project index is corrupt (invalid JSON): {e}"),
            IndexError::NotFound(name) => write!(f, "no project matching '{name}' found"),
        }
    }
}

impl Error for IndexError {}

impl From<io::Error> for IndexError {
    fn from(e: io::Error) -> Self {
        IndexError::Io(e)
    }
}
impl From<serde_json::Error> for IndexError {
    fn from(e: serde_json::Error) -> Self {
        IndexError::Json(e)
    }
}

pub struct ProjectIndex {
    path: PathBuf,
}

impl ProjectIndex {
    /// Opens the same file `project-tracker`'s hook script maintains:
    /// `$HOME/.claude/project-index.json`.
    pub fn open_default() -> Self {
        Self {
            path: aam_core::user_home_dir().join(".claude").join("project-index.json"),
        }
    }

    /// Opens an explicit path -- primarily for tests, so they never touch
    /// a real project index.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn load(&self) -> Result<IndexFile, IndexError> {
        if !self.path.is_file() {
            return Ok(IndexFile::default());
        }
        let text = fs::read_to_string(&self.path)?;
        // `backfill-index.ps1`/`track-session.ps1` both write via
        // PowerShell's `Out-File -Encoding utf8`, which prepends a UTF-8
        // BOM by default -- confirmed on this machine's real
        // project-index.json (`ef bb bf` before the `{`). serde_json
        // doesn't skip it, so strip it here rather than fail to parse a
        // file the hook script itself produced.
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
        Ok(serde_json::from_str(text)?)
    }

    fn save(&self, file: &IndexFile) -> Result<(), IndexError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(file)?;
        aam_core::atomic_write(&self.path, text.as_bytes())?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<ProjectRecord>, IndexError> {
        Ok(self.load()?.projects)
    }

    pub fn get(&self, path: &str) -> Result<Option<ProjectRecord>, IndexError> {
        Ok(self
            .load()?
            .projects
            .into_iter()
            .find(|p| p.path.eq_ignore_ascii_case(path)))
    }

    /// Fuzzy match against `name` and the trailing path segment, case-
    /// insensitive substring match -- `project-tracker`'s `SKILL.md` Mode
    /// 2 matching rule. Returns every match (caller decides how to handle
    /// zero/one/many, same as the skill's own "if ambiguous, list the
    /// candidates and ask" behavior).
    pub fn find_fuzzy(&self, query: &str) -> Result<Vec<ProjectRecord>, IndexError> {
        let query = query.to_lowercase();
        Ok(self
            .load()?
            .projects
            .into_iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&query)
                    || p.path.to_lowercase().contains(&query)
            })
            .collect())
    }

    /// Inserts a new record, or replaces the existing one with the same
    /// `path` (case-insensitive, matching project-tracker's own dedup key).
    pub fn upsert(&self, record: ProjectRecord) -> Result<(), IndexError> {
        let mut file = self.load()?;
        file.projects
            .retain(|p| !p.path.eq_ignore_ascii_case(&record.path));
        file.projects.push(record);
        self.save(&file)
    }

    /// Applies `f` to the record matching `path` (case-insensitive) and
    /// saves the result. Errors with [`IndexError::NotFound`] if no such
    /// record exists.
    pub fn update(
        &self,
        path: &str,
        f: impl FnOnce(&mut ProjectRecord),
    ) -> Result<(), IndexError> {
        let mut file = self.load()?;
        let record = file
            .projects
            .iter_mut()
            .find(|p| p.path.eq_ignore_ascii_case(path))
            .ok_or_else(|| IndexError::NotFound(path.to_string()))?;
        f(record);
        self.save(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_index(label: &str) -> (ProjectIndex, PathBuf) {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "aam-memory-index-test-{label}-{}-{unique}",
            std::process::id()
        ));
        let path = dir.join("project-index.json");
        (ProjectIndex::open(&path), dir)
    }

    fn sample(path: &str, name: &str) -> ProjectRecord {
        ProjectRecord {
            path: path.to_string(),
            name: name.to_string(),
            last_session_id: "sid".into(),
            last_active: "2026-08-08T10:00:00Z".into(),
            created: "2026-08-01T10:00:00Z".into(),
            auto_status: None,
            status_override: None,
            auth_backend: None,
            device_id: "".into(),
            tool_kind: "claude".into(),
            profile_label: None,
            full_sync_enabled: false,
            full_sync_status: None,
            discovery_source: "live".into(),
            sync_approved: true,
        }
    }

    #[test]
    fn upsert_then_list_round_trips() {
        let (index, dir) = temp_index("upsert-list");
        index.upsert(sample("/x/y", "y")).unwrap();
        let projects = index.list().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "y");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn upsert_replaces_existing_record_with_same_path_case_insensitively() {
        let (index, dir) = temp_index("upsert-replace");
        index.upsert(sample("/X/Y", "y")).unwrap();
        let mut updated = sample("/x/y", "y-renamed");
        updated.name = "y-renamed".into();
        index.upsert(updated).unwrap();

        let projects = index.list().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "y-renamed");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn find_fuzzy_matches_name_or_path_substring_case_insensitively() {
        let (index, dir) = temp_index("fuzzy");
        index.upsert(sample("D:\\research\\Gear-Sys", "Gear-Sys")).unwrap();
        index.upsert(sample("D:\\other\\Widget", "Widget")).unwrap();

        assert_eq!(index.find_fuzzy("gear").unwrap().len(), 1);
        assert_eq!(index.find_fuzzy("RESEARCH").unwrap().len(), 1);
        assert_eq!(index.find_fuzzy("nonexistent").unwrap().len(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn update_applies_closure_and_errors_on_missing_path() {
        let (index, dir) = temp_index("update");
        index.upsert(sample("/x/y", "y")).unwrap();

        index
            .update("/x/y", |r| r.status_override = Some("done".into()))
            .unwrap();
        let record = index.get("/x/y").unwrap().unwrap();
        assert_eq!(record.status_override.as_deref(), Some("done"));

        let err = index.update("/nope", |_| {}).unwrap_err();
        assert!(matches!(err, IndexError::NotFound(_)));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn deserializes_project_tracker_own_wrapper_shape() {
        let (index, dir) = temp_index("wrapper-shape");
        fs::create_dir_all(&dir).unwrap();
        // Exactly project-tracker's own on-disk shape: {"projects": [...]}.
        fs::write(
            dir.join("project-index.json"),
            r#"{"projects":[{"path":"/a","name":"a","lastSessionId":"s","lastActive":"t","created":"t","autoStatus":null,"statusOverride":null,"authBackend":null}]}"#,
        )
        .unwrap();

        let projects = index.list().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].path, "/a");
        let _ = fs::remove_dir_all(dir);
    }

    /// Regression test: found by actually running `aam project list`
    /// against this machine's real `project-index.json` (`ef bb bf` before
    /// the `{`) -- `backfill-index.ps1`/`track-session.ps1` both write via
    /// PowerShell's `Out-File -Encoding utf8`, which prepends a UTF-8 BOM
    /// by default. `serde_json` doesn't skip it and fails to parse
    /// otherwise.
    #[test]
    fn deserializes_a_file_with_a_leading_utf8_bom() {
        let (index, dir) = temp_index("bom");
        fs::create_dir_all(&dir).unwrap();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(
            br#"{"projects":[{"path":"/a","name":"a","lastSessionId":"s","lastActive":"t","created":"t","autoStatus":null,"statusOverride":null,"authBackend":null}]}"#,
        );
        fs::write(dir.join("project-index.json"), bytes).unwrap();

        let projects = index.list().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].path, "/a");
        let _ = fs::remove_dir_all(dir);
    }
}
