//! Session discovery/adoption panel (`docs/05-session-memory-bank-module.md`
//! §§5.7-5.9): find sessions on disk that aren't in the Memory-Bank index
//! yet, and explicitly bring them in -- graphical equivalent of
//! `aam session scan/adopt/approve-sync`
//! (`crates/aam-cli/src/commands.rs::run_session`).
//!
//! **Hard constraint, kept visible at all times (not a tooltip)**:
//! scanned/adopted sessions never leave this machine until explicitly
//! approved for sync (`05.7`'s `discoverySource="scan"`/`syncApproved=false`
//! default) -- see the persistent banner in [`view`].

use aam_memory::{DiscoveredSession, ProjectIndex};
use aam_switcher::{Profile, ProfileRegistry, Provider, ProviderRecord, Tool};
use iced::widget::{checkbox, column, container, pick_list, row, text};
use iced::{Element, Length, Task};

use crate::style::{primary_button, secondary_button, SPACING_LG, SPACING_MD};
use crate::task::perform;

#[derive(Debug, Clone)]
pub struct ScannedSession {
    pub session: DiscoveredSession,
    pub profile_label: String,
}

#[derive(Default)]
pub struct State {
    pub discovered: Vec<ScannedSession>,
    pub scanning: bool,
    pub summarize: bool,
    pub summarize_profile: Option<String>,
    pub adopting: bool,
    pub approving: bool,
    pub status_message: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Scan,
    Scanned(Result<Vec<ScannedSession>, String>),
    ToggleSummarize(bool),
    SummarizeProfilePicked(String),
    AdoptAll,
    Adopted(Result<usize, String>),
    ApproveAllScanned,
    Approved(Result<usize, String>),
}

fn scan_all_profiles() -> Result<Vec<ScannedSession>, String> {
    let index = ProjectIndex::open_default();
    let known_ids: Vec<String> = index.list().map_err(|e| e.to_string())?.into_iter().map(|r| r.last_session_id).collect();

    let mut out = Vec::new();
    for profile in ProfileRegistry::open_default().list().map_err(|e| e.to_string())? {
        let discovered = match profile.tool {
            Tool::Claude => aam_memory::scan_claude_sessions(
                &profile.config_dir,
                std::slice::from_ref(&profile.config_dir),
                &known_ids,
            ),
            Tool::Codex => aam_memory::scan_codex_sessions(&profile.config_dir, &known_ids),
        };
        out.extend(discovered.into_iter().map(|session| ScannedSession {
            session,
            profile_label: profile.label.clone(),
        }));
    }
    Ok(out)
}

pub fn update(state: &mut State, message: Message, profiles: &[Profile], providers: &[ProviderRecord]) -> Task<Message> {
    match message {
        Message::Scan => {
            state.scanning = true;
            perform(scan_all_profiles, Message::Scanned)
        }
        Message::Scanned(Ok(found)) => {
            state.scanning = false;
            state.status_message = Some(if found.is_empty() {
                "没有发现新的、还没采集过的会话".to_string()
            } else {
                format!("发现 {} 个未采集的会话", found.len())
            });
            state.discovered = found;
            Task::none()
        }
        Message::Scanned(Err(e)) => {
            state.scanning = false;
            state.status_message = Some(format!("扫描失败: {e}"));
            Task::none()
        }
        Message::ToggleSummarize(v) => {
            state.summarize = v;
            Task::none()
        }
        Message::SummarizeProfilePicked(label) => {
            state.summarize_profile = Some(label);
            Task::none()
        }
        Message::AdoptAll => {
            if state.discovered.is_empty() {
                return Task::none();
            }
            if state.summarize && state.summarize_profile.is_none() {
                state.status_message = Some("勾选了生成摘要，还需要选一个 Profile".to_string());
                return Task::none();
            }
            state.adopting = true;

            let discovered = state.discovered.clone();
            // Resolve down to `(ProviderRecord, api_key)` here (both plain
            // `Send` data) rather than building the actual `Box<dyn
            // Provider>` on this thread and moving it into the closure --
            // `Provider` isn't required to be `Send` (`aam-switcher`'s
            // trait doesn't demand it, and there's no need to widen that
            // just for this), so the object itself has to be constructed
            // *inside* the closure that runs on the worker thread, same
            // as `launch.rs`'s callers already do.
            let provider_source: Option<(ProviderRecord, String)> = if state.summarize {
                state
                    .summarize_profile
                    .as_ref()
                    .and_then(|label| profiles.iter().find(|p| &p.label == label))
                    .and_then(|profile| profile.provider.as_ref())
                    .and_then(|id| providers.iter().find(|p| &p.id == id))
                    .cloned()
                    .and_then(|record| {
                        let key = aam_switcher::provider_secret_store().ok()?.load(&record.id).ok()??;
                        Some((record, key))
                    })
            } else {
                None
            };

            perform(
                move || {
                    let index = ProjectIndex::open_default();
                    let device_id = aam_sync::local_identity(&aam_core::aam_home().join("sync"))
                        .ok()
                        .flatten()
                        .map(|i| i.device_id)
                        .unwrap_or_default();
                    let provider: Option<Box<dyn Provider>> =
                        provider_source.map(|(record, key)| aam_switcher::build_provider(&record, key));

                    let mut adopted = 0;
                    for item in &discovered {
                        let summary = match (&provider, item.session.auto_status.is_none()) {
                            (Some(provider), true) => summarize_session(provider.as_ref(), &item.session).ok(),
                            _ => None,
                        };
                        aam_memory::adopt_session(&index, &item.session, &device_id, &item.profile_label, summary)
                            .map_err(|e| e.to_string())?;
                        adopted += 1;
                    }
                    Ok(adopted)
                },
                Message::Adopted,
            )
        }
        Message::Adopted(Ok(count)) => {
            state.adopting = false;
            state.discovered.clear();
            state.status_message = Some(format!(
                "已采集 {count} 个会话（默认只留在本机，点下面的「批准全部已扫描」才会在下次跨设备同步时被推送）"
            ));
            Task::none()
        }
        Message::Adopted(Err(e)) => {
            state.adopting = false;
            state.status_message = Some(format!("采集失败: {e}"));
            Task::none()
        }
        Message::ApproveAllScanned => {
            state.approving = true;
            perform(
                || {
                    let index = ProjectIndex::open_default();
                    aam_memory::approve_all_scanned(&index).map_err(|e| e.to_string())
                },
                Message::Approved,
            )
        }
        Message::Approved(Ok(count)) => {
            state.approving = false;
            state.status_message = Some(format!("已批准 {count} 条记录参与同步"));
            Task::none()
        }
        Message::Approved(Err(e)) => {
            state.approving = false;
            state.status_message = Some(format!("批准失败: {e}"));
            Task::none()
        }
    }
}

/// Reads a chunk of the session's raw source file and asks `provider`
/// for a one-line summary -- mirrors
/// `crates/aam-cli/src/commands.rs::summarize_session` exactly (same
/// prompt, same 6000-char excerpt), no new business logic.
fn summarize_session(provider: &dyn Provider, session: &DiscoveredSession) -> Result<String, String> {
    const MAX_EXCERPT_CHARS: usize = 6000;
    let content = std::fs::read_to_string(&session.source_file).map_err(|e| e.to_string())?;
    let excerpt: String = content.chars().take(MAX_EXCERPT_CHARS).collect();
    let prompt = format!(
        "以下是一段编程助手会话的原始日志片段（可能是 JSON Lines 格式，忽略格式本身）。请用一句话\
         （20 字以内，中文或英文均可）概括这个会话大致在做什么任务。只输出这一句话，不要任何解释、\
         标点符号以外的其他内容：\n\n{excerpt}"
    );
    provider.complete(&prompt).map(|s| s.trim().to_string()).map_err(|e| e.to_string())
}

pub fn view<'a>(state: &'a State, profiles: &'a [Profile]) -> Element<'a, Message> {
    let constraint_banner = container(
        text(
            "扫描/采集到的会话默认只留在本机，不会自动出现在其他设备 -- 要跨设备同步需要显式点「批准全部已扫描」\
             （真正把内容推送出去的跨设备同步功能在 Round 4 加入）。",
        )
        .size(13),
    )
    .padding(SPACING_MD)
    .width(Length::Fill);

    let mut list = column![].spacing(SPACING_MD);
    if state.discovered.is_empty() {
        list = list.push(text("(点「扫描」发现本机还没采集的会话)"));
    }
    for item in &state.discovered {
        list = list.push(
            row![
                text(item.session.tool_kind).width(Length::Fixed(70.0)),
                text(item.profile_label.clone()).width(Length::Fixed(140.0)),
                text(item.session.path.clone()).width(Length::Fill),
                text(item.session.auto_status.clone().unwrap_or_default()).width(Length::Fixed(200.0)),
            ]
            .spacing(SPACING_MD),
        );
    }

    let summarizable_profiles: Vec<String> = profiles
        .iter()
        .filter(|p| p.provider.is_some())
        .map(|p| p.label.clone())
        .collect();

    let adopt_controls = column![
        row![
            checkbox(state.summarize)
                .label("生成摘要（需要选一个挂了 Provider 的 Profile）")
                .on_toggle(Message::ToggleSummarize),
            pick_list(summarizable_profiles, state.summarize_profile.clone(), Message::SummarizeProfilePicked)
                .placeholder("摘要用的 Profile..."),
        ]
        .spacing(SPACING_MD),
        row![
            secondary_button(
                if state.scanning { "扫描中..." } else { "扫描" },
                if state.scanning { None } else { Some(Message::Scan) }
            ),
            primary_button(
                if state.adopting { "采集中..." } else { "采集全部" },
                if state.adopting || state.discovered.is_empty() { None } else { Some(Message::AdoptAll) }
            ),
            secondary_button(
                if state.approving { "批准中..." } else { "批准全部已扫描" },
                if state.approving { None } else { Some(Message::ApproveAllScanned) }
            ),
        ]
        .spacing(SPACING_MD),
    ]
    .spacing(SPACING_MD);

    let status = state
        .status_message
        .as_ref()
        .map(|m| text(m.clone()))
        .unwrap_or_else(|| text(""));

    container(
        column![
            text("Sessions").size(24),
            constraint_banner,
            iced::widget::scrollable(list).height(Length::FillPortion(3)),
            adopt_controls,
            status,
        ]
        .spacing(SPACING_LG),
    )
    .padding(SPACING_LG)
    .into()
}
