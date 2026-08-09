//! `ProjectRecord`: the schema `~/.claude/skills/project-tracker` already
//! writes to `~/.claude/project-index.json` (its own `SKILL.md` documents
//! the 7 original fields), extended with `docs/05-session-memory-bank-module.md`
//! §5.2's 7 cross-device fields.
//!
//! Every new field is `serde(default)`-compatible with records
//! project-tracker's hook script already wrote *before* this crate
//! existed -- those records have none of the new fields, and must still
//! parse (see `index.rs`'s tests for a fixture in that exact old shape).

use serde::{Deserialize, Serialize};

fn default_tool_kind() -> String {
    "claude".to_string()
}

fn default_discovery_source() -> String {
    "live".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub path: String,
    pub name: String,
    #[serde(rename = "lastSessionId")]
    pub last_session_id: String,
    /// RFC 3339-ish timestamp string, kept as an opaque string (matching
    /// project-tracker's own `.ToString('yyyy-MM-ddTHH:mm:sszzz')`) rather
    /// than parsed -- this crate never needs to do date arithmetic on it,
    /// only display and compare-for-sort.
    #[serde(rename = "lastActive")]
    pub last_active: String,
    pub created: String,
    #[serde(rename = "autoStatus", default)]
    pub auto_status: Option<String>,
    #[serde(rename = "statusOverride", default)]
    pub status_override: Option<String>,
    #[serde(rename = "authBackend", default)]
    pub auth_backend: Option<String>,

    // -- docs/05 §5.2's cross-device fields, all absent from records
    // project-tracker wrote before this crate existed. --
    /// Empty string for records written before this field existed --
    /// there's no way to retroactively know which device wrote them.
    #[serde(rename = "deviceId", default)]
    pub device_id: String,
    /// `"claude"` for pre-Phase-3 records: project-tracker has only ever
    /// tracked Claude Code sessions, so that default is not a guess.
    #[serde(rename = "toolKind", default = "default_tool_kind")]
    pub tool_kind: String,
    #[serde(rename = "profileLabel", default)]
    pub profile_label: Option<String>,
    #[serde(rename = "fullSyncEnabled", default)]
    pub full_sync_enabled: bool,
    #[serde(rename = "fullSyncStatus", default)]
    pub full_sync_status: Option<String>,
    /// `"live"` for pre-Phase-3 records -- they were all written by the
    /// SessionStart/SessionEnd hook, not by `aam session scan`/`adopt`,
    /// which didn't exist yet.
    #[serde(rename = "discoverySource", default = "default_discovery_source")]
    pub discovery_source: String,
    /// `true` for pre-Phase-3 records, matching §5.2's rule that `live`
    /// records default to already-approved-for-sync.
    #[serde(rename = "syncApproved", default = "default_true")]
    pub sync_approved: bool,
    /// Candidate cross-device logical project identity (`docs/08-open-questions-risks.md`
    /// #8) -- `None` by default. This Phase 3b round only adds the field;
    /// nothing yet auto-generates or matches on it, so cross-device
    /// display (`aam project list`) still works by simple concatenation,
    /// not by grouping on this.
    #[serde(rename = "projectId", default)]
    pub project_id: Option<String>,
}

impl ProjectRecord {
    /// `statusOverride` if set, else `autoStatus`, else `None` --
    /// project-tracker's `SKILL.md` display rule.
    pub fn display_status(&self) -> Option<&str> {
        self.status_override
            .as_deref()
            .or(self.auto_status.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_a_pre_phase3_record_with_only_the_original_seven_fields() {
        // Exactly the shape project-tracker's track-session.ps1/backfill-index.ps1
        // write today -- no Phase 3 fields at all.
        let json = r#"{
            "path": "D:\\research\\Gear-Sys",
            "name": "Gear-Sys",
            "lastSessionId": "286cce60-aaaa",
            "lastActive": "2026-08-08T10:15:00+08:00",
            "created": "2026-07-01T09:00:00+08:00",
            "autoStatus": "验证研究工作文档并核实问题",
            "statusOverride": null,
            "authBackend": "oauth-subscription"
        }"#;

        let record: ProjectRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.device_id, "");
        assert_eq!(record.tool_kind, "claude");
        assert_eq!(record.profile_label, None);
        assert!(!record.full_sync_enabled);
        assert_eq!(record.full_sync_status, None);
        assert_eq!(record.discovery_source, "live");
        assert!(record.sync_approved);
    }

    #[test]
    fn round_trips_a_full_phase3_record() {
        let record = ProjectRecord {
            path: "/home/x".into(),
            name: "x".into(),
            last_session_id: "sid".into(),
            last_active: "2026-08-08T10:00:00Z".into(),
            created: "2026-08-01T10:00:00Z".into(),
            auto_status: Some("auto".into()),
            status_override: Some("override".into()),
            auth_backend: Some("oauth-subscription".into()),
            device_id: "device-1".into(),
            tool_kind: "codex".into(),
            profile_label: Some("work".into()),
            full_sync_enabled: true,
            full_sync_status: Some("ok".into()),
            discovery_source: "scan".into(),
            sync_approved: false,
            project_id: Some("proj-1".into()),
        };
        let json = serde_json::to_string(&record).unwrap();
        let parsed: ProjectRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, record);
    }

    #[test]
    fn display_status_prefers_override_then_auto_then_none() {
        let mut record = ProjectRecord {
            path: "p".into(),
            name: "n".into(),
            last_session_id: "s".into(),
            last_active: "t".into(),
            created: "t".into(),
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
            project_id: None,
        };
        assert_eq!(record.display_status(), None);

        record.auto_status = Some("auto".into());
        assert_eq!(record.display_status(), Some("auto"));

        record.status_override = Some("override".into());
        assert_eq!(record.display_status(), Some("override"));
    }
}
