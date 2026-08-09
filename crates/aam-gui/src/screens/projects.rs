//! Project browser (`docs/05-session-memory-bank-module.md`): browse the
//! local + cross-device Memory-Bank index, resume a project by actually
//! opening a terminal that runs `claude --resume`/`codex resume` (`06.5`
//! -- the GUI owns the terminal it opens), and manually link two records
//! under one cross-device `projectId` (`08` #8). Graphical equivalent of
//! `aam project list/show/resume/link`
//! (`crates/aam-cli/src/commands.rs::run_project`).
//!
//! Follows the Phase 4 Round 2 plan's "用户友好性" principles: the main
//! list shows only what's needed to decide whether to hit "接续"
//! (name, tool·Profile badge, last active, can-resume-here status);
//! every other field lives behind a per-row "详情" toggle, and
//! resume/link feedback is shown on the row it belongs to, not a single
//! shared status line -- these are potentially-concurrent per-row
//! actions, unlike Profiles/Providers' one-thing-at-a-time forms.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use aam_memory::{ProjectIndex, ProjectRecord};
use aam_switcher::{Profile, ProviderRecord, Tool};
use iced::widget::{column, container, pick_list, row, scrollable, text, text_input};
use iced::{Element, Length, Task};

use crate::style::{primary_button, secondary_button, SPACING_LG, SPACING_MD, SPACING_SM};
use crate::task::perform;

#[derive(Debug, Clone)]
pub enum ResumeStatus {
    Opening,
    Failed(String),
}

#[derive(Default)]
pub struct State {
    pub records: Vec<ProjectRecord>,
    pub query: String,
    pub expanded: HashSet<String>,
    pub resume_status: HashMap<String, ResumeStatus>,
    /// Path -> chosen Profile label, present while that row's "接续
    /// （内嵌）" confirm-and-maybe-override UI (`06.4` step 3: "上次用的
    /// Profile 是 X，是否沿用？" -- also allows changing it) is open for
    /// that row. Removed once confirmed or cancelled.
    pub pending_embedded: HashMap<String, String>,
    pub link_path_a: String,
    pub link_path_b: String,
    pub link_status: Option<String>,
    pub linking: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Result<Vec<ProjectRecord>, String>),
    QueryChanged(String),
    ToggleExpanded(String),
    Resume(String),
    Resumed(String, Result<(), String>),
    /// Opens the "上次用的 Profile 是 X，是否沿用？" confirm/override row
    /// for this record (`06.4` step 3) -- purely local UI state, no
    /// terminal involved yet.
    ResumeEmbeddedRequested(String),
    /// User picked a different local Profile than the recorded one,
    /// while the confirm row for `path` is open.
    ResumeEmbeddedProfileChanged(String, String),
    ResumeEmbeddedCancelled(String),
    /// Confirmed (with whatever Profile ended up chosen) -- handled
    /// entirely by `app::update` (needs `state.terminal`), see the
    /// no-op arm in [`update`].
    ResumeEmbeddedConfirmed(String),
    LinkPathAChanged(String),
    LinkPathBChanged(String),
    SubmitLink,
    Linked(Result<String, String>),
}

fn local_and_mirrored() -> Result<Vec<ProjectRecord>, String> {
    let mut all = ProjectIndex::open_default().list().map_err(|e| e.to_string())?;
    all.extend(aam_memory::remote_mirror_index().list().map_err(|e| e.to_string())?);
    all.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    Ok(all)
}

pub fn load() -> Task<Message> {
    perform(local_and_mirrored, Message::Loaded)
}

/// The tool a record's Profile lookup needs -- `ProjectRecord::tool_kind`
/// is a plain string (`05.2`'s on-disk schema), `"codex"` or anything
/// else (including legacy records that predate the field) means Claude.
///
/// `pub(crate)`: `app.rs` reuses this for the embedded-tab resume path
/// (Phase 5 Round 2) rather than re-deriving the same tool_kind check.
pub(crate) fn record_tool(record: &ProjectRecord) -> Tool {
    if record.tool_kind == "codex" {
        Tool::Codex
    } else {
        Tool::Claude
    }
}

/// The actual resume command line for a record -- shared by both the
/// external-window path (below) and the embedded-tab path (`app.rs`,
/// Phase 5 Round 2), so they never drift apart.
pub(crate) fn resume_command(tool: Tool, last_session_id: &str) -> String {
    match tool {
        Tool::Codex => format!("codex resume {last_session_id}"),
        Tool::Claude => format!("claude --resume {last_session_id}"),
    }
}

pub fn update(state: &mut State, message: Message, profiles: &[Profile], providers: &[ProviderRecord]) -> Task<Message> {
    match message {
        Message::Loaded(Ok(records)) => {
            state.records = records;
            Task::none()
        }
        Message::Loaded(Err(e)) => {
            state.link_status = Some(format!("加载项目列表失败: {e}"));
            Task::none()
        }
        Message::QueryChanged(q) => {
            state.query = q;
            Task::none()
        }
        Message::ToggleExpanded(path) => {
            if !state.expanded.remove(&path) {
                state.expanded.insert(path);
            }
            Task::none()
        }
        Message::Resume(path) => {
            let record = state.records.iter().find(|r| r.path == path).cloned();
            let Some(record) = record else {
                return Task::none();
            };
            state.resume_status.insert(path.clone(), ResumeStatus::Opening);

            let tool = record_tool(&record);
            let profile = record
                .profile_label
                .as_ref()
                .and_then(|label| profiles.iter().find(|p| p.tool == tool && &p.label == label).cloned());
            // `resumable(...)` (used by `view`) already guarantees this
            // path is only reachable when the directory exists and a
            // local Profile was found -- `update` re-derives the same
            // checks rather than trusting the button was only enabled
            // correctly, since `Task`s can in principle be in flight
            // across state changes.
            let Some(profile) = profile else {
                state.resume_status.insert(
                    path.clone(),
                    ResumeStatus::Failed("这条记录没有关联的本机 Profile，无法确定用哪个账号接续".to_string()),
                );
                return Task::none();
            };
            if !Path::new(&record.path).is_dir() {
                state
                    .resume_status
                    .insert(path.clone(), ResumeStatus::Failed("本机未找到这个目录".to_string()));
                return Task::none();
            }

            let providers = providers.to_vec();
            let command = resume_command(tool, &record.last_session_id);
            let project_path = record.path.clone();
            perform(
                move || {
                    let env = crate::launch::launch_env(tool, &profile, &providers);
                    crate::terminal::open_terminal(Some(Path::new(&project_path)), &env, &command)
                        .map(|_child| ())
                        .map_err(|e| e.to_string())
                },
                move |result| Message::Resumed(path.clone(), result),
            )
        }
        Message::ResumeEmbeddedRequested(path) => {
            let default_label = state
                .records
                .iter()
                .find(|r| r.path == path)
                .and_then(|r| r.profile_label.clone())
                .unwrap_or_default();
            state.pending_embedded.insert(path, default_label);
            Task::none()
        }
        Message::ResumeEmbeddedProfileChanged(path, label) => {
            state.pending_embedded.insert(path, label);
            Task::none()
        }
        Message::ResumeEmbeddedCancelled(path) => {
            state.pending_embedded.remove(&path);
            Task::none()
        }
        // `app::update` intercepts this variant before it ever reaches
        // here (it needs `state.terminal`, a sibling screen this module
        // has no access to) -- kept as a no-op arm purely so this match
        // stays exhaustive.
        Message::ResumeEmbeddedConfirmed(_) => Task::none(),
        Message::Resumed(path, Ok(())) => {
            state.resume_status.remove(&path);
            Task::none()
        }
        Message::Resumed(path, Err(e)) => {
            state.resume_status.insert(path, ResumeStatus::Failed(e));
            Task::none()
        }
        Message::LinkPathAChanged(v) => {
            state.link_path_a = v;
            Task::none()
        }
        Message::LinkPathBChanged(v) => {
            state.link_path_b = v;
            Task::none()
        }
        Message::SubmitLink => {
            if state.link_path_a.trim().is_empty() || state.link_path_b.trim().is_empty() {
                state.link_status = Some("两个路径都要填".to_string());
                return Task::none();
            }
            state.linking = true;
            let a = state.link_path_a.clone();
            let b = state.link_path_b.clone();
            perform(
                move || {
                    let local = ProjectIndex::open_default();
                    let mirror = aam_memory::remote_mirror_index();
                    aam_memory::link_projects(&local, &mirror, &a, &b).map_err(|e| e.to_string())
                },
                Message::Linked,
            )
        }
        Message::Linked(Ok(project_id)) => {
            state.linking = false;
            state.link_status = Some(format!("已关联，projectId = {project_id}"));
            state.link_path_a.clear();
            state.link_path_b.clear();
            load()
        }
        Message::Linked(Err(e)) => {
            state.linking = false;
            state.link_status = Some(format!("关联失败: {e}"));
            Task::none()
        }
    }
}

/// Whether "接续" should be clickable for `record` right now, and if not,
/// the human-readable reason to show instead of the button (design
/// principle #3: no raw error dumps).
///
/// `pub(crate)`: `app.rs` reuses this same gate for the embedded-tab
/// resume path (Phase 5 Round 2) -- one set of rules for "can this
/// record be resumed right now", not two.
pub(crate) fn resumable(record: &ProjectRecord, profiles: &[Profile]) -> Result<(), String> {
    if !Path::new(&record.path).is_dir() {
        let device_note = if record.device_id.is_empty() {
            String::new()
        } else {
            format!("（记录设备 id: {}）", record.device_id)
        };
        return Err(format!("本机未找到目录{device_note}"));
    }
    let tool = record_tool(record);
    match &record.profile_label {
        None => Err("没有关联的 Profile".to_string()),
        Some(label) => {
            if profiles.iter().any(|p| p.tool == tool && &p.label == label) {
                Ok(())
            } else {
                Err(format!("Profile '{label}' 未在本机注册"))
            }
        }
    }
}

/// Case-insensitive substring match against name or path -- same filter
/// `commands.rs::ProjectAction::Show`/`Resume` do inline, pulled out here
/// so it's unit-testable independent of `iced`.
fn filter_records<'a>(records: &'a [ProjectRecord], query: &str) -> Vec<&'a ProjectRecord> {
    let query_lower = query.to_lowercase();
    records
        .iter()
        .filter(|r| {
            query_lower.is_empty()
                || r.name.to_lowercase().contains(&query_lower)
                || r.path.to_lowercase().contains(&query_lower)
        })
        .collect()
}

pub fn view<'a>(state: &'a State, profiles: &'a [Profile], _providers: &'a [ProviderRecord]) -> Element<'a, Message> {
    let filtered = filter_records(&state.records, &state.query);

    let mut list = column![].spacing(SPACING_MD);
    if state.records.is_empty() {
        list = list.push(text("(还没有已采集的项目 -- 去 Sessions 页扫描/采纳一些)"));
    } else if filtered.is_empty() {
        list = list.push(text("没有匹配的项目"));
    }
    for record in filtered {
        list = list.push(project_card(state, record, profiles));
    }

    let link_form = column![
        text("手动关联两条记录（跨设备 projectId，`08` #8）").size(14),
        row![
            text_input("路径 A", &state.link_path_a).on_input(Message::LinkPathAChanged),
            text_input("路径 B", &state.link_path_b).on_input(Message::LinkPathBChanged),
            primary_button(
                if state.linking { "关联中..." } else { "关联" },
                if state.linking { None } else { Some(Message::SubmitLink) }
            ),
        ]
        .spacing(SPACING_SM),
        state
            .link_status
            .as_ref()
            .map(|m| text(m.clone()))
            .unwrap_or_else(|| text("")),
    ]
    .spacing(SPACING_SM);

    container(
        column![
            text("Projects").size(24),
            text_input("按名字/路径搜索...", &state.query).on_input(Message::QueryChanged),
            scrollable(list).height(Length::FillPortion(3)),
            link_form,
        ]
        .spacing(SPACING_LG),
    )
    .padding(SPACING_LG)
    .into()
}

fn project_card<'a>(state: &'a State, record: &'a ProjectRecord, profiles: &'a [Profile]) -> Element<'a, Message> {
    let dot = if Path::new(&record.path).is_dir() { "🟢" } else { "⚪" };
    let profile_badge = format!(
        "{} · {}",
        record.tool_kind,
        record.profile_label.as_deref().unwrap_or("(无 Profile)")
    );

    let resume_area: Element<'_, Message> = match state.resume_status.get(&record.path) {
        Some(ResumeStatus::Opening) => text("正在打开终端...").into(),
        Some(ResumeStatus::Failed(e)) => row![
            text(e.clone()),
            secondary_button("重试", Some(Message::Resume(record.path.clone()))),
        ]
        .spacing(SPACING_SM)
        .into(),
        None if state.pending_embedded.contains_key(&record.path) => {
            // `06.4` step 3: "上次用的 Profile 是 X，是否沿用？" -- also
            // lets the user pick a different local Profile of the same
            // tool before confirming, rather than silently trusting the
            // recorded one.
            let tool = record_tool(record);
            let chosen = state.pending_embedded.get(&record.path).cloned();
            let candidates: Vec<String> = profiles.iter().filter(|p| p.tool == tool).map(|p| p.label.clone()).collect();
            row![
                text("沿用 Profile:").size(12),
                pick_list(candidates, chosen, {
                    let path = record.path.clone();
                    move |label| Message::ResumeEmbeddedProfileChanged(path.clone(), label)
                }),
                primary_button("确认", Some(Message::ResumeEmbeddedConfirmed(record.path.clone()))),
                secondary_button("取消", Some(Message::ResumeEmbeddedCancelled(record.path.clone()))),
            ]
            .spacing(SPACING_SM)
            .into()
        }
        None => match resumable(record, profiles) {
            Ok(()) => row![
                primary_button("接续", Some(Message::Resume(record.path.clone()))),
                // Phase 5 Round 2: same eligibility check, just opens an
                // embedded tab instead of an external window -- both
                // stay available side by side, external isn't removed.
                secondary_button("接续（内嵌）", Some(Message::ResumeEmbeddedRequested(record.path.clone()))),
            ]
            .spacing(SPACING_SM)
            .into(),
            Err(reason) => row![
                text(reason).size(12),
                primary_button("接续", None), // visibly present but disabled -- not hidden, so it's clear *why* nothing happens
            ]
            .spacing(SPACING_SM)
            .into(),
        },
    };

    let header = row![
        text(dot),
        text(record.name.clone()).size(16).width(Length::Fixed(200.0)),
        text(profile_badge).width(Length::Fixed(220.0)),
        text(record.last_active.clone()).width(Length::Fixed(200.0)),
        resume_area,
        secondary_button("详情", Some(Message::ToggleExpanded(record.path.clone()))),
    ]
    .spacing(SPACING_MD)
    .align_y(iced::Alignment::Center);

    let mut card = column![header].spacing(SPACING_SM);
    if state.expanded.contains(&record.path) {
        card = card.push(
            column![
                text(format!("path: {}", record.path)).size(12),
                text(format!("created: {}", record.created)).size(12),
                text(format!(
                    "status: {}",
                    record.display_status().unwrap_or("(尚无记录)")
                ))
                .size(12),
                text(format!("discovery: {}", record.discovery_source)).size(12),
                text(format!("sync approved: {}", record.sync_approved)).size(12),
                text(format!(
                    "device id: {}",
                    if record.device_id.is_empty() { "-" } else { &record.device_id }
                ))
                .size(12),
                text(format!("project id: {}", record.project_id.as_deref().unwrap_or("-"))).size(12),
                auth_backend_note(record),
            ]
            .spacing(2),
        );
    }

    container(card).padding(SPACING_MD).width(Length::Fill).into()
}

fn auth_backend_note(record: &ProjectRecord) -> Element<'_, Message> {
    match &record.auth_backend {
        Some(backend) if backend != "oauth-subscription" => text(format!(
            "注意：上次通过 '{backend}' 使用，不是官方订阅 -- 用不同账号接续 extended-thinking 会话可能报签名错误"
        ))
        .size(12)
        .into(),
        _ => text("").into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(path: &str, name: &str) -> ProjectRecord {
        ProjectRecord {
            path: path.to_string(),
            name: name.to_string(),
            last_session_id: "sess-1".to_string(),
            last_active: "2026-08-09T00:00:00+00:00".to_string(),
            created: "2026-08-01T00:00:00+00:00".to_string(),
            auto_status: None,
            status_override: None,
            auth_backend: Some("oauth-subscription".to_string()),
            device_id: String::new(),
            tool_kind: "claude".to_string(),
            profile_label: None,
            full_sync_enabled: false,
            full_sync_status: None,
            discovery_source: "live".to_string(),
            sync_approved: true,
            project_id: None,
        }
    }

    fn sample_profile(tool: Tool, label: &str) -> Profile {
        Profile {
            label: label.to_string(),
            tool,
            config_dir: std::path::PathBuf::from("C:\\fake\\config"),
            provider: None,
        }
    }

    #[test]
    fn filter_records_empty_query_returns_everything() {
        let records = vec![sample_record("/a", "alpha"), sample_record("/b", "beta")];
        assert_eq!(filter_records(&records, "").len(), 2);
    }

    #[test]
    fn filter_records_matches_name_case_insensitively() {
        let records = vec![sample_record("/a", "Alpha"), sample_record("/b", "Beta")];
        let found = filter_records(&records, "ALPHA");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Alpha");
    }

    #[test]
    fn filter_records_matches_path_too() {
        let records = vec![sample_record("/projects/widget", "x"), sample_record("/other", "y")];
        let found = filter_records(&records, "widget");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "/projects/widget");
    }

    #[test]
    fn record_tool_defaults_to_claude() {
        let mut record = sample_record("/a", "a");
        record.tool_kind = "claude".to_string();
        assert_eq!(record_tool(&record), Tool::Claude);
        record.tool_kind = "something-unrecognized".to_string();
        assert_eq!(record_tool(&record), Tool::Claude);
    }

    #[test]
    fn record_tool_recognizes_codex() {
        let mut record = sample_record("/a", "a");
        record.tool_kind = "codex".to_string();
        assert_eq!(record_tool(&record), Tool::Codex);
    }

    #[test]
    fn resumable_rejects_a_missing_local_directory() {
        let mut record = sample_record("Z:\\definitely\\does\\not\\exist\\anywhere", "x");
        record.profile_label = Some("work".to_string());
        record.device_id = "device-a".to_string();
        let profiles = vec![sample_profile(Tool::Claude, "work")];
        let err = resumable(&record, &profiles).unwrap_err();
        assert!(err.contains("本机未找到目录"));
        assert!(err.contains("device-a"));
    }

    #[test]
    fn resumable_rejects_a_record_with_no_profile_label() {
        let base = TempDirGuard::new();
        let mut record = sample_record(base.path_str(), "x");
        record.profile_label = None;
        let err = resumable(&record, &[]).unwrap_err();
        assert!(err.contains("没有关联的 Profile"));
    }

    #[test]
    fn resumable_rejects_a_profile_not_registered_locally() {
        let base = TempDirGuard::new();
        let mut record = sample_record(base.path_str(), "x");
        record.profile_label = Some("ghost".to_string());
        let err = resumable(&record, &[]).unwrap_err();
        assert!(err.contains("未在本机注册"));
    }

    #[test]
    fn resumable_accepts_an_existing_dir_with_a_registered_profile() {
        let base = TempDirGuard::new();
        let mut record = sample_record(base.path_str(), "x");
        record.profile_label = Some("work".to_string());
        let profiles = vec![sample_profile(Tool::Claude, "work")];
        assert!(resumable(&record, &profiles).is_ok());
    }

    /// Real (but always-exists) temp directory, since `resumable` checks
    /// `Path::is_dir()` against the filesystem -- not worth a full
    /// dependency-injection rewrite of `resumable` just for this.
    struct TempDirGuard(std::path::PathBuf);

    impl TempDirGuard {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "aam-gui-projects-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDirGuard(dir)
        }

        fn path_str(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
