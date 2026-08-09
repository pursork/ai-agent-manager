//! Profile management screen (`docs/03-credential-account-module.md`):
//! list known Profiles, create new ones, verify login status, assign a
//! Provider, and open a terminal under a Profile's environment -- the
//! graphical equivalent of `aam profile list/add/verify/use-provider`
//! (`crates/aam-cli/src/commands.rs::run_profile`).
//!
//! Doesn't own a copy of the Providers list -- `app.rs` passes
//! `&[ProviderRecord]` in from `providers::State` on every `view`/relevant
//! `update` call (Phase 4 Round 2 plan, design item 2: avoids every
//! screen that needs it maintaining its own mirror + sync message).

use std::collections::HashMap;

use aam_switcher::{claude_backend, codex_backend, ApplyCodexProvider, Profile, ProfileRegistry, ProviderRecord, Tool};
use iced::widget::{column, container, pick_list, row, text, text_input};
use iced::{Element, Length, Task};

use crate::style::{primary_button, secondary_button, SPACING_LG, SPACING_MD, SPACING_SM};
use crate::task::perform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyStatus {
    Unknown,
    Checking,
    LoggedIn,
    NotLoggedIn,
    Error,
}

pub struct State {
    pub profiles: Vec<Profile>,
    pub verify_status: HashMap<(Tool, String), VerifyStatus>,
    pub new_label: String,
    pub new_tool: Tool,
    pub status_message: Option<String>,
    pub creating: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            profiles: Vec::new(),
            verify_status: HashMap::new(),
            new_label: String::new(),
            // `Tool` is defined in `aam_switcher`, so this crate can't add
            // a `Default` impl for it (orphan rule) -- pick a starting
            // value here instead.
            new_tool: Tool::Claude,
            status_message: None,
            creating: false,
        }
    }
}

impl State {
    fn key(tool: Tool, label: &str) -> (Tool, String) {
        (tool, label.to_string())
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Result<Vec<Profile>, String>),
    NewLabelChanged(String),
    NewToolChanged(Tool),
    SubmitNew,
    Created(Result<Profile, String>),
    Verify(Tool, String),
    VerifyResult(Tool, String, Result<bool, String>),
    OpenTerminal(Tool, String),
    TerminalOpened(Result<(), String>),
    /// Open into an embedded terminal tab instead of an external window
    /// (Phase 5 Round 2). Handled entirely by `app::update` (needs
    /// `state.terminal`) -- see the no-op arm in [`update`].
    OpenEmbedded(Tool, String),
    AssignProvider(Tool, String, String),
    ProviderAssigned(Result<(String, Tool), String>),
}

pub fn load() -> Task<Message> {
    perform(|| ProfileRegistry::open_default().list().map_err(|e| e.to_string()), Message::Loaded)
}

pub fn update(state: &mut State, message: Message, providers: &[ProviderRecord]) -> Task<Message> {
    match message {
        Message::Loaded(Ok(profiles)) => {
            state.profiles = profiles;
            Task::none()
        }
        Message::Loaded(Err(e)) => {
            state.status_message = Some(format!("加载 Profile 列表失败: {e}"));
            Task::none()
        }
        Message::NewLabelChanged(v) => {
            state.new_label = v;
            Task::none()
        }
        Message::NewToolChanged(tool) => {
            state.new_tool = tool;
            Task::none()
        }
        Message::SubmitNew => {
            if state.new_label.trim().is_empty() {
                state.status_message = Some("label 不能为空".to_string());
                return Task::none();
            }
            state.creating = true;
            let tool = state.new_tool;
            let label = state.new_label.clone();
            perform(
                move || {
                    let registry = ProfileRegistry::open_default();
                    match tool {
                        Tool::Claude => claude_backend::create_profile(&registry, &label).map_err(|e| e.to_string()),
                        Tool::Codex => codex_backend::create_profile(&registry, &label).map_err(|e| e.to_string()),
                    }
                },
                Message::Created,
            )
        }
        Message::Created(Ok(profile)) => {
            state.creating = false;
            state.new_label.clear();
            state.status_message = Some(format!(
                "已创建 {} Profile '{}'，接下来点这一行的「打开终端」完成交互式登录",
                profile.tool, profile.label
            ));
            state.profiles.push(profile);
            Task::none()
        }
        Message::Created(Err(e)) => {
            state.creating = false;
            state.status_message = Some(format!("创建 Profile 失败: {e}"));
            Task::none()
        }
        Message::Verify(tool, label) => {
            state.verify_status.insert(State::key(tool, &label), VerifyStatus::Checking);
            let profile = state.profiles.iter().find(|p| p.tool == tool && p.label == label).cloned();
            let Some(profile) = profile else {
                return Task::none();
            };
            perform(
                move || {
                    let result = match tool {
                        Tool::Claude => claude_backend::verify_login(&profile),
                        Tool::Codex => codex_backend::verify_login(&profile),
                    };
                    result.map_err(|e| e.to_string())
                },
                move |result| Message::VerifyResult(tool, label.clone(), result),
            )
        }
        Message::VerifyResult(tool, label, result) => {
            let status = match result {
                Ok(true) => VerifyStatus::LoggedIn,
                Ok(false) => VerifyStatus::NotLoggedIn,
                Err(e) => {
                    state.status_message = Some(format!("验证 {label} 失败: {e}"));
                    VerifyStatus::Error
                }
            };
            state.verify_status.insert(State::key(tool, &label), status);
            Task::none()
        }
        Message::OpenTerminal(tool, label) => open_terminal_for(state, tool, label, providers),
        Message::TerminalOpened(Ok(())) => Task::none(),
        Message::TerminalOpened(Err(e)) => {
            state.status_message = Some(format!("打开终端失败: {e}"));
            Task::none()
        }
        Message::OpenEmbedded(..) => Task::none(),
        Message::AssignProvider(tool, label, provider_id) => {
            let profile = state.profiles.iter().find(|p| p.tool == tool && p.label == label).cloned();
            let provider_record = providers.iter().find(|p| p.id == provider_id).cloned();
            let (Some(profile), Some(record)) = (profile, provider_record) else {
                return Task::none();
            };
            perform(
                move || {
                    let registry = ProfileRegistry::open_default();
                    let key = aam_switcher::provider_secret_store()
                        .map_err(|e| e.to_string())?
                        .load(&record.id)
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| format!("provider '{}' 没有保存的 API key", record.id))?;
                    let provider_obj = aam_switcher::build_provider(&record, key);
                    match tool {
                        Tool::Claude => {
                            claude_backend::apply_provider(&registry, &profile, provider_obj.as_ref())
                                .map_err(|e| e.to_string())?;
                        }
                        Tool::Codex => {
                            let mut op = ApplyCodexProvider::new(profile.config_dir.clone(), provider_obj.as_ref());
                            aam_core::execute(&mut op).map_err(|e| e.to_string())?;
                            registry
                                .set_provider(tool, &profile.label, Some(record.id.clone()))
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    Ok((profile.label, tool))
                },
                Message::ProviderAssigned,
            )
        }
        Message::ProviderAssigned(Ok((label, tool))) => {
            state.status_message = Some(format!("已为 {tool} Profile '{label}' 挂载 Provider"));
            // Re-fetch the whole list so this row's `profile.provider`
            // (which the pick_list reads to show what's selected) reflects
            // the write that just succeeded.
            load()
        }
        Message::ProviderAssigned(Err(e)) => {
            state.status_message = Some(format!("挂载 Provider 失败: {e}"));
            Task::none()
        }
    }
}

/// Opens a terminal already set up for this Profile (env vars for
/// `CLAUDE_CONFIG_DIR`/`CODEX_HOME` + any attached Provider, `03.6`),
/// running the bare `claude`/`codex` command -- used both for day-to-day
/// use of an already-logged-in Profile and, right after creating one, as
/// the place to complete the interactive login (`aam-gui` only opens the
/// terminal; the login prompt inside it is 100% user-driven, never
/// scripted).
fn open_terminal_for(state: &mut State, tool: Tool, label: String, providers: &[ProviderRecord]) -> Task<Message> {
    let profile = state.profiles.iter().find(|p| p.tool == tool && p.label == label).cloned();
    let Some(profile) = profile else {
        return Task::none();
    };
    let providers = providers.to_vec();
    perform(
        move || {
            let env = crate::launch::launch_env(tool, &profile, &providers);
            crate::terminal::open_terminal(None, &env, tool.as_str())
                .map(|_child| ())
                .map_err(|e| e.to_string())
        },
        Message::TerminalOpened,
    )
}

pub fn view<'a>(state: &'a State, providers: &'a [ProviderRecord]) -> Element<'a, Message> {
    let mut list = column![].spacing(SPACING_MD);
    if state.profiles.is_empty() {
        list = list.push(text("(还没有 Profile，用下面的表单新建一个)"));
    }
    for profile in &state.profiles {
        let key = (profile.tool, profile.label.clone());
        let status = state.verify_status.get(&key).copied().unwrap_or(VerifyStatus::Unknown);
        let status_text = match status {
            VerifyStatus::Unknown => "未验证".to_string(),
            VerifyStatus::Checking => "验证中...".to_string(),
            VerifyStatus::LoggedIn => "已登录".to_string(),
            VerifyStatus::NotLoggedIn => "未登录".to_string(),
            VerifyStatus::Error => "验证出错".to_string(),
        };
        let provider_ids: Vec<String> = providers.iter().map(|p| p.id.clone()).collect();
        let assign_row = row![
            pick_list(provider_ids, profile.provider.clone(), {
                let tool = profile.tool;
                let label = profile.label.clone();
                move |chosen| Message::AssignProvider(tool, label.clone(), chosen)
            })
            .placeholder("挂载 Provider..."),
        ]
        .spacing(SPACING_SM);

        let row_el = row![
            text(profile.tool.as_str()).width(Length::Fixed(70.0)),
            text(profile.label.clone()).width(Length::Fixed(140.0)),
            text(profile.config_dir.display().to_string()).width(Length::Fill),
            text(status_text).width(Length::Fixed(80.0)),
            secondary_button("验证", Some(Message::Verify(profile.tool, profile.label.clone()))),
            primary_button("打开终端", Some(Message::OpenTerminal(profile.tool, profile.label.clone()))),
            secondary_button("打开终端（内嵌）", Some(Message::OpenEmbedded(profile.tool, profile.label.clone()))),
            assign_row,
        ]
        .spacing(SPACING_MD)
        .align_y(iced::Alignment::Center);

        list = list.push(row_el);
    }

    let new_profile_form = column![
        text("新建 Profile").size(18),
        row![
            secondary_button("Claude", Some(Message::NewToolChanged(Tool::Claude))),
            secondary_button("Codex", Some(Message::NewToolChanged(Tool::Codex))),
            text(format!("当前选择: {}", state.new_tool)),
        ]
        .spacing(SPACING_MD),
        text_input("label", &state.new_label).on_input(Message::NewLabelChanged),
        primary_button(
            if state.creating { "创建中..." } else { "创建" },
            if state.creating { None } else { Some(Message::SubmitNew) }
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
            text("Profiles").size(24),
            scrollable_list(list),
            new_profile_form,
            status,
        ]
        .spacing(SPACING_LG),
    )
    .padding(SPACING_LG)
    .into()
}

fn scrollable_list(content: iced::widget::Column<'_, Message>) -> Element<'_, Message> {
    iced::widget::scrollable(content).height(Length::FillPortion(3)).into()
}
