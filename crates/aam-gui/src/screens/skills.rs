//! Skills management panel (`docs/09-skills-management.md`): the
//! graphical equivalent of the full `aam skills` CLI surface --
//! list/status/scan/adopt (local move-in + git clone)/install-bundled/
//! check-updates/update (`crates/aam-cli/src/commands.rs::run_skills`).
//!
//! Follows the same "用户友好性" principles Round 2 established
//! (information layering, one primary action per row, human error copy,
//! row-scoped feedback for row-scoped actions, a shared status line for
//! whole-screen batch actions) -- see `docs/06-gui-terminal-shell.md` §6.8.

use std::collections::HashMap;
use std::path::PathBuf;

use aam_skills::{DiscoveredSkill, InstallOutcome, SkillEntry, SkillsIndex, UpdateStatus, BUNDLED_SKILLS};
use aam_switcher::{Profile, Tool};
use iced::widget::{checkbox, column, container, row, text, text_input};
use iced::{Element, Length, Task};

use crate::style::{primary_button, secondary_button, SPACING_LG, SPACING_MD, SPACING_SM};
use crate::task::perform;

#[derive(Debug, Clone)]
pub enum RowStatus {
    Working,
    Failed(String),
}

/// `aam_skills::ManagedSkill` isn't `Clone` (no callers needed it before
/// this screen), but `iced::Message` needs to be -- rather than widen
/// that struct's derive for one GUI-side use, mirror just the fields
/// this screen renders into a local, `Clone`-able type.
#[derive(Debug, Clone)]
pub struct ManagedSkillRow {
    pub name: String,
    pub canonical_path: PathBuf,
    pub linked_to_codex: bool,
    pub is_git_repo: bool,
}

impl From<aam_skills::ManagedSkill> for ManagedSkillRow {
    fn from(m: aam_skills::ManagedSkill) -> Self {
        Self {
            name: m.name,
            canonical_path: m.canonical_path,
            linked_to_codex: m.linked_to_codex,
            is_git_repo: m.is_git_repo,
        }
    }
}

pub struct GitAdoptForm {
    pub name: String,
    pub source: String,
    /// `"manual"` or `"auto"` -- a plain string matching `SkillEntry`'s
    /// own on-disk representation, same reasoning as the CLI's
    /// `--update-mode` flag.
    pub update_mode: String,
}

impl Default for GitAdoptForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            source: String::new(),
            update_mode: "manual".to_string(),
        }
    }
}

#[derive(Default)]
pub struct State {
    pub managed: Vec<ManagedSkillRow>,
    pub index_entries: HashMap<String, SkillEntry>,
    pub discovered: Vec<DiscoveredSkill>,
    pub update_statuses: HashMap<String, UpdateStatus>,
    pub row_status: HashMap<String, RowStatus>,
    pub scanning: bool,
    pub checking_updates: bool,
    pub updating_all_auto: bool,
    pub install_force: bool,
    pub git_form: GitAdoptForm,
    pub adopting_git: bool,
    pub status_message: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Result<(Vec<ManagedSkillRow>, Vec<SkillEntry>), String>),
    Scan,
    Scanned(Result<Vec<DiscoveredSkill>, String>),
    AdoptLocal(String),
    LocalAdopted(String, Result<(), String>),
    GitNameChanged(String),
    GitSourceChanged(String),
    GitUpdateModeChanged(String),
    SubmitGitAdopt,
    GitAdopted(Result<String, String>),
    InstallForceToggled(bool),
    InstallBundled(&'static str),
    BundledInstalled(&'static str, Result<InstallOutcome, String>),
    ShareWithCodex(String),
    Shared(String, Result<(), String>),
    CheckUpdates,
    UpdatesChecked(Result<Vec<UpdateStatus>, String>),
    UpdateSkill(String),
    SkillUpdated(String, Result<(), String>),
    UpdateAllAuto,
    AllAutoUpdated(Result<Vec<AutoUpdateOutcome>, String>),
}

/// One skill's outcome from `update --all-auto` -- name plus whether it
/// updated cleanly. Named the same way `aam-skills::AutoUpdateOutcome`
/// is, for the same reason (avoids a `clippy::type_complexity` warning
/// on the inline tuple-in-a-Vec-in-a-Result shape).
type AutoUpdateOutcome = (String, Result<(), String>);

/// Search locations for "not yet adopted" skills: the canonical store
/// itself (content that predates the index), Codex's own dir, and any
/// Claude Profile whose `skills/` isn't already linked back to canonical
/// -- mirrors `crates/aam-cli/src/commands.rs::skills_search_dirs`
/// exactly (same reasoning: scanning an already-linked Profile's `skills/`
/// would just re-report canonical's own content under a different label).
fn skills_search_dirs(profiles: &[Profile]) -> Vec<(String, PathBuf)> {
    let canonical = aam_skills::claude_personal_skills_dir();
    let mut dirs = vec![
        ("claude-canonical".to_string(), canonical.clone()),
        ("codex".to_string(), aam_skills::codex_user_skills_dir()),
    ];
    for profile in profiles.iter().filter(|p| p.tool == Tool::Claude) {
        let profile_skills_dir = profile.config_dir.join("skills");
        if !aam_skills::resolves_to(&profile_skills_dir, &canonical) {
            dirs.push((format!("claude:{}", profile.label), profile_skills_dir));
        }
    }
    dirs
}

pub fn load() -> Task<Message> {
    perform(
        || {
            let managed: Vec<ManagedSkillRow> = aam_skills::list_managed_skills()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(ManagedSkillRow::from)
                .collect();
            let entries = SkillsIndex::open_default().list().map_err(|e| e.to_string())?;
            Ok((managed, entries))
        },
        Message::Loaded,
    )
}

pub fn update(state: &mut State, message: Message, profiles: &[Profile]) -> Task<Message> {
    match message {
        Message::Loaded(Ok((managed, entries))) => {
            state.managed = managed;
            state.index_entries = entries.into_iter().map(|e| (e.name.clone(), e)).collect();
            Task::none()
        }
        Message::Loaded(Err(e)) => {
            state.status_message = Some(format!("加载 skills 列表失败: {e}"));
            Task::none()
        }
        Message::Scan => {
            state.scanning = true;
            let known_names: Vec<String> = state.index_entries.keys().cloned().collect();
            let search_dirs = skills_search_dirs(profiles);
            perform(
                move || {
                    let canonical = aam_skills::claude_personal_skills_dir();
                    aam_skills::scan_unmanaged_skills(&known_names, &canonical, &search_dirs).map_err(|e| e.to_string())
                },
                Message::Scanned,
            )
        }
        Message::Scanned(Ok(found)) => {
            state.scanning = false;
            state.status_message = Some(if found.is_empty() {
                "没有发现新的、还没纳管的 skill".to_string()
            } else {
                format!("发现 {} 个未纳管的 skill", found.len())
            });
            state.discovered = found;
            Task::none()
        }
        Message::Scanned(Err(e)) => {
            state.scanning = false;
            state.status_message = Some(format!("扫描失败: {e}"));
            Task::none()
        }
        Message::AdoptLocal(name) => {
            state.row_status.insert(name.clone(), RowStatus::Working);
            let search_dirs = skills_search_dirs(profiles);
            let name_for_perform = name.clone();
            perform(
                move || {
                    let index = SkillsIndex::open_default();
                    aam_skills::adopt_local_skill(&index, &name_for_perform, &search_dirs).map_err(|e| e.to_string())
                },
                move |result| Message::LocalAdopted(name.clone(), result),
            )
        }
        Message::LocalAdopted(name, Ok(())) => {
            state.row_status.remove(&name);
            state.discovered.retain(|d| d.name != name);
            state.status_message = Some(format!("'{name}' 已纳管"));
            load()
        }
        Message::LocalAdopted(name, Err(e)) => {
            state.row_status.insert(name, RowStatus::Failed(e));
            Task::none()
        }
        Message::GitNameChanged(v) => {
            state.git_form.name = v;
            Task::none()
        }
        Message::GitSourceChanged(v) => {
            state.git_form.source = v;
            Task::none()
        }
        Message::GitUpdateModeChanged(v) => {
            state.git_form.update_mode = v;
            Task::none()
        }
        Message::SubmitGitAdopt => {
            if state.git_form.name.trim().is_empty() || state.git_form.source.trim().is_empty() {
                state.status_message = Some("name 和 source 都不能为空".to_string());
                return Task::none();
            }
            state.adopting_git = true;
            let name = state.git_form.name.clone();
            let spec = state.git_form.source.clone();
            let update_mode = state.git_form.update_mode.clone();
            perform(
                move || {
                    let (url, git_ref) = aam_skills::parse_source_spec(&spec);
                    let index = SkillsIndex::open_default();
                    aam_skills::adopt_from_git(&index, &name, &url, git_ref.as_deref(), &update_mode)
                        .map_err(|e| e.to_string())?;
                    Ok(name)
                },
                Message::GitAdopted,
            )
        }
        Message::GitAdopted(Ok(name)) => {
            state.adopting_git = false;
            state.status_message = Some(format!("已从 git 引入 '{name}'"));
            state.git_form = GitAdoptForm::default();
            load()
        }
        Message::GitAdopted(Err(e)) => {
            state.adopting_git = false;
            state.status_message = Some(format!("从 git 引入失败: {e}"));
            Task::none()
        }
        Message::InstallForceToggled(v) => {
            state.install_force = v;
            Task::none()
        }
        Message::InstallBundled(name) => {
            state.row_status.insert(name.to_string(), RowStatus::Working);
            let force = state.install_force;
            perform(
                move || aam_skills::install_bundled_skill(name, force).map_err(|e| e.to_string()),
                move |result| Message::BundledInstalled(name, result),
            )
        }
        Message::BundledInstalled(name, Ok(outcome)) => {
            state.row_status.remove(name);
            state.status_message = Some(install_outcome_message(name, outcome));
            load()
        }
        Message::BundledInstalled(name, Err(e)) => {
            state.row_status.insert(name.to_string(), RowStatus::Failed(e));
            Task::none()
        }
        Message::ShareWithCodex(name) => {
            state.row_status.insert(name.clone(), RowStatus::Working);
            let name_for_perform = name.clone();
            perform(
                move || {
                    aam_skills::share_skill_with_codex(&name_for_perform).map_err(|e| e.to_string())?;
                    SkillsIndex::open_default()
                        .record_share_target(&name_for_perform, "codex")
                        .map_err(|e| e.to_string())?;
                    Ok(())
                },
                move |result| Message::Shared(name.clone(), result),
            )
        }
        Message::Shared(name, Ok(())) => {
            state.row_status.remove(&name);
            state.status_message = Some(format!("'{name}' 已分享到 Codex"));
            load()
        }
        Message::Shared(name, Err(e)) => {
            state.row_status.insert(name, RowStatus::Failed(e));
            Task::none()
        }
        Message::CheckUpdates => {
            state.checking_updates = true;
            perform(
                || {
                    let index = SkillsIndex::open_default();
                    aam_skills::check_updates(&index).map_err(|e| e.to_string())
                },
                Message::UpdatesChecked,
            )
        }
        Message::UpdatesChecked(Ok(statuses)) => {
            state.checking_updates = false;
            let available: usize = statuses.iter().filter(|s| !s.up_to_date).count();
            state.status_message = Some(if statuses.is_empty() {
                "没有 git 来源的 skill 需要检查".to_string()
            } else {
                format!("检查了 {} 个 git 来源的 skill，{} 个有更新", statuses.len(), available)
            });
            state.update_statuses = statuses.into_iter().map(|s| (s.name.clone(), s)).collect();
            Task::none()
        }
        Message::UpdatesChecked(Err(e)) => {
            state.checking_updates = false;
            state.status_message = Some(format!("检查更新失败: {e}"));
            Task::none()
        }
        Message::UpdateSkill(name) => {
            state.row_status.insert(name.clone(), RowStatus::Working);
            let name_for_perform = name.clone();
            perform(
                move || {
                    let index = SkillsIndex::open_default();
                    aam_skills::update_skill(&index, &name_for_perform).map_err(|e| e.to_string())
                },
                move |result| Message::SkillUpdated(name.clone(), result),
            )
        }
        Message::SkillUpdated(name, Ok(())) => {
            state.row_status.remove(&name);
            if let Some(status) = state.update_statuses.get_mut(&name) {
                status.up_to_date = true;
            }
            state.status_message = Some(format!("'{name}' 已更新到最新"));
            Task::none()
        }
        Message::SkillUpdated(name, Err(e)) => {
            state.row_status.insert(name, RowStatus::Failed(e));
            Task::none()
        }
        Message::UpdateAllAuto => {
            state.updating_all_auto = true;
            perform(
                || {
                    let index = SkillsIndex::open_default();
                    let outcomes = aam_skills::update_all_auto(&index).map_err(|e| e.to_string())?;
                    Ok(outcomes.into_iter().map(|(name, r)| (name, r.map_err(|e| e.to_string()))).collect())
                },
                Message::AllAutoUpdated,
            )
        }
        Message::AllAutoUpdated(Ok(outcomes)) => {
            state.updating_all_auto = false;
            if outcomes.is_empty() {
                state.status_message = Some("没有 update-mode=auto 的 skill".to_string());
            } else {
                let summary = outcomes
                    .iter()
                    .map(|(name, r)| match r {
                        Ok(()) => format!("{name}: 已更新"),
                        Err(e) => format!("{name}: 失败 ({e})"),
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                state.status_message = Some(summary);
            }
            Task::none()
        }
        Message::AllAutoUpdated(Err(e)) => {
            state.updating_all_auto = false;
            state.status_message = Some(format!("批量更新失败: {e}"));
            Task::none()
        }
    }
}

/// `InstallOutcome`'s three states, in plain language -- pulled out so
/// it's unit-testable without going through `iced`/a real install.
fn install_outcome_message(name: &str, outcome: InstallOutcome) -> String {
    match outcome {
        InstallOutcome::Installed => format!("'{name}' 已安装"),
        InstallOutcome::AlreadyUpToDate => format!("'{name}' 已是最新，无需改动"),
        InstallOutcome::Overwritten => format!("'{name}' 已用最新内容覆盖"),
    }
}

/// What the "更新" button should show/do for a managed skill, given its
/// index entry (do we even know a source for it?) and the last
/// `check-updates` result (do we know if it's current?). Pulled out so
/// the row-rendering logic in `view` stays simple and this stays
/// unit-testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateButtonState {
    /// `source == "local"` or not tracked at all -- no update concept
    /// applies, don't show anything.
    NotApplicable,
    /// Git-sourced, but `check-updates` hasn't run yet (or hasn't run
    /// since this skill was adopted).
    UnknownNeedsCheck,
    UpToDate,
    UpdateAvailable,
}

fn update_button_state(entry: Option<&SkillEntry>, status: Option<&UpdateStatus>) -> UpdateButtonState {
    match entry {
        None => UpdateButtonState::NotApplicable,
        Some(e) if e.source == "local" => UpdateButtonState::NotApplicable,
        Some(_) => match status {
            None => UpdateButtonState::UnknownNeedsCheck,
            Some(s) if s.up_to_date => UpdateButtonState::UpToDate,
            Some(_) => UpdateButtonState::UpdateAvailable,
        },
    }
}

pub fn view<'a>(state: &'a State) -> Element<'a, Message> {
    let managed_count = state.managed.len();
    let codex_linked_count = state.managed.iter().filter(|m| m.linked_to_codex).count();
    let summary = text(format!("{managed_count} 个已纳管 skill，{codex_linked_count} 个已链接到 Codex"));

    let mut managed_list = column![].spacing(SPACING_MD);
    if state.managed.is_empty() {
        managed_list = managed_list.push(text("(还没有纳管任何 skill)"));
    }
    for skill in &state.managed {
        managed_list = managed_list.push(managed_row(state, skill));
    }

    let mut discovered_list = column![].spacing(SPACING_MD);
    if state.discovered.is_empty() {
        discovered_list = discovered_list.push(text("(点「扫描」发现本机还没纳管的 skill)"));
    }
    for d in &state.discovered {
        discovered_list = discovered_list.push(discovered_row(state, d));
    }

    let git_form = &state.git_form;
    let git_adopt_form = column![
        text("从 Git 引入新 Skill").size(16),
        text("会执行真实的 git clone，需要网络").size(12),
        text_input("name", &git_form.name).on_input(Message::GitNameChanged),
        text_input("<git-url>[@ref]", &git_form.source).on_input(Message::GitSourceChanged),
        row![
            secondary_button("manual", Some(Message::GitUpdateModeChanged("manual".to_string()))),
            secondary_button("auto", Some(Message::GitUpdateModeChanged("auto".to_string()))),
            text(format!("update-mode: {}", git_form.update_mode)),
        ]
        .spacing(SPACING_MD),
        primary_button(
            if state.adopting_git { "引入中..." } else { "引入" },
            if state.adopting_git { None } else { Some(Message::SubmitGitAdopt) }
        ),
    ]
    .spacing(SPACING_SM);

    let mut bundled_list = column![].spacing(SPACING_SM);
    for bundled in BUNDLED_SKILLS {
        let working = matches!(state.row_status.get(bundled.name), Some(RowStatus::Working));
        bundled_list = bundled_list.push(
            row![
                text(bundled.name).width(Length::Fixed(200.0)),
                secondary_button(
                    if working { "安装中..." } else { "安装" },
                    if working { None } else { Some(Message::InstallBundled(bundled.name)) }
                ),
            ]
            .spacing(SPACING_MD),
        );
    }
    let install_bundled_section = column![
        text("安装内置 Skill").size(16),
        checkbox(state.install_force).label("强制覆盖已有内容").on_toggle(Message::InstallForceToggled),
        bundled_list,
    ]
    .spacing(SPACING_SM);

    let update_controls = row![
        secondary_button(
            if state.checking_updates { "检查中..." } else { "检查更新" },
            if state.checking_updates { None } else { Some(Message::CheckUpdates) }
        ),
        secondary_button(
            if state.updating_all_auto { "更新中..." } else { "全部自动更新" },
            if state.updating_all_auto { None } else { Some(Message::UpdateAllAuto) }
        ),
    ]
    .spacing(SPACING_MD);

    let status = state
        .status_message
        .as_ref()
        .map(|m| text(m.clone()))
        .unwrap_or_else(|| text(""));

    container(
        column![
            text("Skills").size(24),
            summary,
            update_controls,
            iced::widget::scrollable(managed_list).height(Length::FillPortion(2)),
            secondary_button(
                if state.scanning { "扫描中..." } else { "扫描" },
                if state.scanning { None } else { Some(Message::Scan) }
            ),
            iced::widget::scrollable(discovered_list).height(Length::FillPortion(2)),
            git_adopt_form,
            install_bundled_section,
            status,
        ]
        .spacing(SPACING_LG),
    )
    .padding(SPACING_LG)
    .into()
}

fn managed_row<'a>(state: &'a State, skill: &'a ManagedSkillRow) -> Element<'a, Message> {
    let entry = state.index_entries.get(&skill.name);
    let update_status = state.update_statuses.get(&skill.name);
    let row_status = state.row_status.get(&skill.name);

    let mut actions = row![
        column![
            text(skill.name.clone()),
            text(skill.canonical_path.display().to_string()).size(10),
        ]
        .width(Length::Fixed(220.0)),
    ]
    .spacing(SPACING_SM);

    if skill.is_git_repo {
        // `09.2`: a git-repo canonical store syncs itself via
        // `git push`/`pull`, `aam` doesn't touch its content -- just a
        // quiet badge, not an action (there's nothing to click here).
        actions = actions.push(text("git 仓库").size(11));
    }

    if !skill.linked_to_codex {
        let working = matches!(row_status, Some(RowStatus::Working));
        actions = actions.push(secondary_button(
            if working { "分享中..." } else { "分享到 Codex" },
            if working { None } else { Some(Message::ShareWithCodex(skill.name.clone())) },
        ));
    } else {
        actions = actions.push(text("已链接 Codex").size(12));
    }

    match update_button_state(entry, update_status) {
        UpdateButtonState::NotApplicable => {}
        UpdateButtonState::UnknownNeedsCheck => actions = actions.push(text("未知（点「检查更新」）").size(12)),
        UpdateButtonState::UpToDate => actions = actions.push(text("已是最新").size(12)),
        UpdateButtonState::UpdateAvailable => {
            let working = matches!(row_status, Some(RowStatus::Working));
            actions = actions.push(primary_button(
                if working { "更新中..." } else { "更新" },
                if working { None } else { Some(Message::UpdateSkill(skill.name.clone())) },
            ));
        }
    }

    if let Some(RowStatus::Failed(e)) = row_status {
        actions = actions.push(text(e.clone()).size(12));
    }

    actions.into()
}

fn discovered_row<'a>(state: &'a State, skill: &'a DiscoveredSkill) -> Element<'a, Message> {
    let row_status = state.row_status.get(&skill.name);
    let working = matches!(row_status, Some(RowStatus::Working));

    let mut r = row![
        text(skill.name.clone()).width(Length::Fixed(180.0)),
        text(skill.location.clone()).width(Length::Fixed(140.0)),
        text(skill.path.display().to_string()).width(Length::Fill),
        primary_button(
            if working { "纳管中..." } else { "纳管" },
            if working { None } else { Some(Message::AdoptLocal(skill.name.clone())) }
        ),
    ]
    .spacing(SPACING_MD);

    if let Some(RowStatus::Failed(e)) = row_status {
        r = r.push(text(e.clone()).size(12));
    }

    r.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, source: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_string(),
            managed: true,
            share_targets: Vec::new(),
            source: source.to_string(),
            update_mode: "manual".to_string(),
        }
    }

    fn status(name: &str, up_to_date: bool) -> UpdateStatus {
        UpdateStatus {
            name: name.to_string(),
            up_to_date,
            local_commit: "aaa".to_string(),
            upstream_commit: "bbb".to_string(),
        }
    }

    #[test]
    fn update_button_state_not_applicable_when_unindexed() {
        assert_eq!(update_button_state(None, None), UpdateButtonState::NotApplicable);
    }

    #[test]
    fn update_button_state_not_applicable_for_local_source() {
        let e = entry("x", "local");
        assert_eq!(update_button_state(Some(&e), None), UpdateButtonState::NotApplicable);
    }

    #[test]
    fn update_button_state_needs_check_for_git_source_never_checked() {
        let e = entry("x", "https://example.com/x.git");
        assert_eq!(update_button_state(Some(&e), None), UpdateButtonState::UnknownNeedsCheck);
    }

    #[test]
    fn update_button_state_up_to_date() {
        let e = entry("x", "https://example.com/x.git");
        let s = status("x", true);
        assert_eq!(update_button_state(Some(&e), Some(&s)), UpdateButtonState::UpToDate);
    }

    #[test]
    fn update_button_state_update_available() {
        let e = entry("x", "https://example.com/x.git");
        let s = status("x", false);
        assert_eq!(update_button_state(Some(&e), Some(&s)), UpdateButtonState::UpdateAvailable);
    }

    #[test]
    fn install_outcome_message_covers_all_three_states() {
        assert!(install_outcome_message("x", InstallOutcome::Installed).contains("已安装"));
        assert!(install_outcome_message("x", InstallOutcome::AlreadyUpToDate).contains("已是最新"));
        assert!(install_outcome_message("x", InstallOutcome::Overwritten).contains("已用最新内容覆盖"));
    }

    #[test]
    fn skills_search_dirs_always_includes_canonical_and_codex() {
        let dirs = skills_search_dirs(&[]);
        let labels: Vec<&str> = dirs.iter().map(|(l, _)| l.as_str()).collect();
        assert!(labels.contains(&"claude-canonical"));
        assert!(labels.contains(&"codex"));
        assert_eq!(dirs.len(), 2);
    }

    #[test]
    fn skills_search_dirs_ignores_codex_profiles() {
        let profiles = vec![Profile {
            label: "work".to_string(),
            tool: Tool::Codex,
            config_dir: PathBuf::from("C:\\fake\\codex-work"),
            provider: None,
        }];
        let dirs = skills_search_dirs(&profiles);
        // Codex Profiles don't have a `skills/` link-consistency concept
        // the way Claude Profiles do (`03.7`) -- only the fixed "codex"
        // entry (already present) should represent Codex, not a
        // per-Profile one.
        assert_eq!(dirs.len(), 2);
    }
}
