//! Persistent skills ledger (`docs/09-skills-management.md` §9.5):
//! `~/.claude/skills/.aam-skills-index.json`, next to the canonical store
//! itself. Tracks which skills `aam-skills` actually manages, where
//! they've been explicitly shared, and (for skills adopted from a git
//! source) what that source is -- none of which a plain directory scan
//! (`list_managed_skills`) can know on its own.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

/// One entry in the index (`09.5`'s schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillEntry {
    pub name: String,
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub share_targets: Vec<String>,
    /// `"local"` (default) or `"<git-url>[@ref]"`.
    #[serde(default = "default_source")]
    pub source: String,
    /// `"manual"` (default) or `"auto"`.
    #[serde(default = "default_update_mode")]
    pub update_mode: String,
}

fn default_source() -> String {
    "local".to_string()
}

fn default_update_mode() -> String {
    "manual".to_string()
}

impl SkillEntry {
    /// A freshly-adopted, locally-authored skill: `source: "local"`,
    /// `update_mode: "manual"`, no share targets yet.
    pub fn new_local(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            managed: true,
            share_targets: Vec::new(),
            source: default_source(),
            update_mode: default_update_mode(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct IndexFile {
    #[serde(default)]
    skills: Vec<SkillEntry>,
}

#[derive(Debug)]
pub enum IndexError {
    Io(io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IndexError::Io(e) => write!(f, "skills index I/O error: {e}"),
            IndexError::Json(e) => write!(f, "skills index is corrupt (invalid JSON): {e}"),
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

pub struct SkillsIndex {
    path: PathBuf,
}

impl SkillsIndex {
    /// Opens the index next to the canonical skills store
    /// (`crate::claude_personal_skills_dir().join(".aam-skills-index.json")`).
    pub fn open_default() -> Self {
        Self {
            path: crate::claude_personal_skills_dir().join(".aam-skills-index.json"),
        }
    }

    /// Opens an explicit path -- primarily for tests, so they never touch
    /// a real skills store.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn load(&self) -> Result<IndexFile, IndexError> {
        if !self.path.is_file() {
            return Ok(IndexFile::default());
        }
        let text = fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&text)?)
    }

    fn save(&self, file: &IndexFile) -> Result<(), IndexError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(file)?;
        aam_core::atomic_write(&self.path, text.as_bytes())?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<SkillEntry>, IndexError> {
        Ok(self.load()?.skills)
    }

    pub fn get(&self, name: &str) -> Result<Option<SkillEntry>, IndexError> {
        Ok(self.load()?.skills.into_iter().find(|s| s.name == name))
    }

    /// Inserts a new entry, or replaces the existing one with the same
    /// `name`.
    pub fn upsert(&self, entry: SkillEntry) -> Result<(), IndexError> {
        let mut file = self.load()?;
        file.skills.retain(|s| s.name != entry.name);
        file.skills.push(entry);
        self.save(&file)
    }

    /// Adds `target` to `name`'s `share_targets` (deduplicated), creating
    /// a `local`/`manual` entry first if `name` isn't tracked yet --
    /// `aam skills adopt --share-with` calls this after each successful
    /// share, per `09.5`'s "`shareTargets` 累加记录，不覆盖".
    pub fn record_share_target(&self, name: &str, target: &str) -> Result<(), IndexError> {
        let mut entry = self.get(name)?.unwrap_or_else(|| SkillEntry::new_local(name));
        if !entry.share_targets.iter().any(|t| t == target) {
            entry.share_targets.push(target.to_string());
        }
        self.upsert(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_index(label: &str) -> (SkillsIndex, PathBuf) {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "aam-skills-index-test-{label}-{}-{unique}",
            std::process::id()
        ));
        (SkillsIndex::open(dir.join(".aam-skills-index.json")), dir)
    }

    #[test]
    fn upsert_then_list_round_trips() {
        let (index, dir) = temp_index("upsert-list");
        index.upsert(SkillEntry::new_local("pdf-processing")).unwrap();
        let skills = index.list().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "pdf-processing");
        assert_eq!(skills[0].source, "local");
        assert_eq!(skills[0].update_mode, "manual");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn upsert_replaces_existing_entry_with_same_name() {
        let (index, dir) = temp_index("upsert-replace");
        index.upsert(SkillEntry::new_local("x")).unwrap();
        let mut updated = SkillEntry::new_local("x");
        updated.share_targets = vec!["codex".to_string()];
        index.upsert(updated).unwrap();

        let skills = index.list().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].share_targets, vec!["codex".to_string()]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn record_share_target_dedups_and_creates_entry_if_missing() {
        let (index, dir) = temp_index("share-target");
        index.record_share_target("new-skill", "codex").unwrap();
        index.record_share_target("new-skill", "codex").unwrap(); // dedup
        index.record_share_target("new-skill", "claude:work").unwrap();

        let entry = index.get("new-skill").unwrap().unwrap();
        assert_eq!(entry.share_targets, vec!["codex".to_string(), "claude:work".to_string()]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn deserializes_an_entry_missing_newer_fields() {
        let (index, dir) = temp_index("legacy");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(".aam-skills-index.json"),
            r#"{"skills":[{"name":"old-skill","managed":true}]}"#,
        )
        .unwrap();

        let skills = index.list().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source, "local");
        assert_eq!(skills[0].update_mode, "manual");
        assert!(skills[0].share_targets.is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
