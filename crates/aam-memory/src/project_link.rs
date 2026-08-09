//! Manual cross-device project identity linking (`docs/08-open-questions-risks.md`
//! #8): `aam project link <path-a> <path-b>` tells aam "these two records
//! -- possibly from different devices, possibly from the local index and
//! the cross-device mirror -- are the same logical project", by giving
//! them the same `projectId`.
//!
//! No automatic matching: same-name/same-path heuristics across devices
//! have real false-positive potential (two different projects that
//! happen to share a folder name), so this crate never guesses --
//! `aam project list`/`show` still just concatenate local + mirrored
//! records rather than grouping on `projectId` themselves (that's a
//! future display enhancement once this mechanism has actually been used
//! for a while).

use crate::index::{IndexError, ProjectIndex};
use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum LinkError {
    Index(IndexError),
    NotFound(String),
    /// Both records already carry a `projectId`, and they disagree --
    /// refuses to silently pick one and discard the other's grouping.
    Conflict {
        path_a: String,
        id_a: String,
        path_b: String,
        id_b: String,
    },
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::Index(e) => write!(f, "{e}"),
            LinkError::NotFound(path) => {
                write!(f, "no project record for '{path}' found (checked both the local index and the cross-device mirror)")
            }
            LinkError::Conflict { path_a, id_a, path_b, id_b } => write!(
                f,
                "'{path_a}' already has projectId '{id_a}' and '{path_b}' already has a different \
                 one ('{id_b}') -- resolve which grouping is correct before linking"
            ),
        }
    }
}

impl Error for LinkError {}

impl From<IndexError> for LinkError {
    fn from(e: IndexError) -> Self {
        LinkError::Index(e)
    }
}

/// Which of the two indices a located record actually lives in, so the
/// update can be written back to the right place -- a mirror record's
/// `projectId` update must never land in the local `project-index.json`.
enum Location<'a> {
    Local(&'a ProjectIndex),
    Mirror(&'a ProjectIndex),
}

fn locate<'a>(local: &'a ProjectIndex, mirror: &'a ProjectIndex, path: &str) -> Result<Location<'a>, LinkError> {
    if local.get(path)?.is_some() {
        return Ok(Location::Local(local));
    }
    if mirror.get(path)?.is_some() {
        return Ok(Location::Mirror(mirror));
    }
    Err(LinkError::NotFound(path.to_string()))
}

fn index_of<'a>(loc: &Location<'a>) -> &'a ProjectIndex {
    match loc {
        Location::Local(i) | Location::Mirror(i) => i,
    }
}

/// Links `path_a` and `path_b` to the same `projectId`, returning it.
/// Each path is looked up in `local` first, then `mirror`. Picks the
/// shared id per this module's doc comment's rule (adopt whichever side
/// already has one; generate a fresh UUID if neither does; error if both
/// have different ones already).
pub fn link_projects(
    local: &ProjectIndex,
    mirror: &ProjectIndex,
    path_a: &str,
    path_b: &str,
) -> Result<String, LinkError> {
    let loc_a = locate(local, mirror, path_a)?;
    let loc_b = locate(local, mirror, path_b)?;

    let record_a = index_of(&loc_a).get(path_a)?.expect("just located");
    let record_b = index_of(&loc_b).get(path_b)?.expect("just located");

    let project_id = match (&record_a.project_id, &record_b.project_id) {
        (Some(a), Some(b)) if a != b => {
            return Err(LinkError::Conflict {
                path_a: path_a.to_string(),
                id_a: a.clone(),
                path_b: path_b.to_string(),
                id_b: b.clone(),
            });
        }
        (Some(a), _) => a.clone(),
        (_, Some(b)) => b.clone(),
        (None, None) => uuid::Uuid::new_v4().to_string(),
    };

    index_of(&loc_a).update(path_a, |r| r.project_id = Some(project_id.clone()))?;
    index_of(&loc_b).update(path_b, |r| r.project_id = Some(project_id.clone()))?;

    Ok(project_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::ProjectRecord;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_index(label: &str) -> (ProjectIndex, PathBuf) {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "aam-memory-link-test-{label}-{}-{unique}",
            std::process::id()
        ));
        (ProjectIndex::open(dir.join("index.json")), dir)
    }

    fn sample(path: &str, project_id: Option<&str>) -> ProjectRecord {
        ProjectRecord {
            path: path.to_string(),
            name: path.to_string(),
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
            project_id: project_id.map(str::to_string),
        }
    }

    #[test]
    fn links_two_unlinked_records_with_a_fresh_id() {
        let (local, local_dir) = temp_index("fresh-local");
        let (mirror, mirror_dir) = temp_index("fresh-mirror");
        local.upsert(sample("/a", None)).unwrap();
        mirror.upsert(sample("/b", None)).unwrap();

        let id = link_projects(&local, &mirror, "/a", "/b").unwrap();
        assert_eq!(local.get("/a").unwrap().unwrap().project_id, Some(id.clone()));
        assert_eq!(mirror.get("/b").unwrap().unwrap().project_id, Some(id));

        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(mirror_dir);
    }

    #[test]
    fn adopts_the_existing_id_when_only_one_side_has_one() {
        let (local, local_dir) = temp_index("one-side-local");
        let (mirror, mirror_dir) = temp_index("one-side-mirror");
        local.upsert(sample("/a", Some("proj-existing"))).unwrap();
        mirror.upsert(sample("/b", None)).unwrap();

        let id = link_projects(&local, &mirror, "/a", "/b").unwrap();
        assert_eq!(id, "proj-existing");
        assert_eq!(
            mirror.get("/b").unwrap().unwrap().project_id.as_deref(),
            Some("proj-existing")
        );

        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(mirror_dir);
    }

    #[test]
    fn conflicting_existing_ids_are_rejected() {
        let (local, local_dir) = temp_index("conflict-local");
        let (mirror, mirror_dir) = temp_index("conflict-mirror");
        local.upsert(sample("/a", Some("proj-1"))).unwrap();
        mirror.upsert(sample("/b", Some("proj-2"))).unwrap();

        let err = link_projects(&local, &mirror, "/a", "/b").unwrap_err();
        assert!(matches!(err, LinkError::Conflict { .. }));
        // Neither side should have been touched.
        assert_eq!(local.get("/a").unwrap().unwrap().project_id.as_deref(), Some("proj-1"));
        assert_eq!(mirror.get("/b").unwrap().unwrap().project_id.as_deref(), Some("proj-2"));

        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(mirror_dir);
    }

    #[test]
    fn missing_path_errors() {
        let (local, local_dir) = temp_index("missing-local");
        let (mirror, mirror_dir) = temp_index("missing-mirror");
        local.upsert(sample("/a", None)).unwrap();

        let err = link_projects(&local, &mirror, "/a", "/does-not-exist").unwrap_err();
        assert!(matches!(err, LinkError::NotFound(p) if p == "/does-not-exist"));

        let _ = fs::remove_dir_all(local_dir);
        let _ = fs::remove_dir_all(mirror_dir);
    }
}
