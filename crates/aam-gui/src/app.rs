//! Top-level `iced` application state: owns which screen is active and
//! routes messages/tasks to it. Per `docs/06-gui-terminal-shell.md` §6.2,
//! this file (and its `screens::*` children) contain **no business
//! logic** of their own -- every actual operation is a call into
//! `aam-switcher`/`aam-memory`/`aam-sync`'s already-CLI-tested public API.
//!
//! `profiles`/`providers` are the single owners of their own lists;
//! `projects`/`sessions` don't keep mirrored copies (Phase 4 Round 2
//! plan, design item 2) -- `update`/`view` just borrow
//! `&state.profiles.profiles`/`&state.providers.providers` for the
//! duration of the call.

use std::path::Path;

use iced::widget::{button, column, container, row, text};
use iced::{Element, Length, Subscription, Task};

use crate::screens::{embedded_terminal, profiles, projects, providers, sessions, skills, sync};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Profiles,
    Providers,
    Projects,
    Sessions,
    Skills,
    Sync,
    Terminal,
}

pub struct State {
    screen: Screen,
    profiles: profiles::State,
    providers: providers::State,
    projects: projects::State,
    sessions: sessions::State,
    skills: skills::State,
    sync: sync::State,
    terminal: embedded_terminal::State,
    /// Whether `wt.exe` was found at startup -- checked once; the user
    /// would need to restart `aam-gui` after installing it anyway for a
    /// fresh PATH to take effect, so there's no point re-checking live.
    wt_available: bool,
    wt_banner_dismissed: bool,
    wt_install_status: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            screen: Screen::Profiles,
            profiles: profiles::State::default(),
            providers: providers::State::default(),
            projects: projects::State::default(),
            sessions: sessions::State::default(),
            skills: skills::State::default(),
            sync: sync::State::default(),
            terminal: embedded_terminal::State::default(),
            wt_available: crate::terminal::wt_available(),
            wt_banner_dismissed: false,
            wt_install_status: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SwitchScreen(Screen),
    Profiles(profiles::Message),
    Providers(providers::Message),
    Projects(projects::Message),
    Sessions(sessions::Message),
    Skills(skills::Message),
    Sync(sync::Message),
    Terminal(embedded_terminal::Message),
    DismissWtBanner,
    InstallWindowsTerminal,
    WindowsTerminalInstallTriggered(Result<(), String>),
}

pub fn new() -> (State, Task<Message>) {
    (
        State::default(),
        Task::batch([
            profiles::load().map(Message::Profiles),
            providers::load().map(Message::Providers),
            projects::load().map(Message::Projects),
            skills::load().map(Message::Skills),
        ]),
    )
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::SwitchScreen(screen) => {
            state.screen = screen;
            Task::none()
        }
        // Intercepted before falling through to `profiles::update` --
        // opening an embedded tab needs `state.terminal`, a sibling
        // screen `profiles.rs` has no access to (Phase 5 Round 2 plan,
        // design item 3). `profiles::update` still has a no-op arm for
        // this variant purely so its own `match` stays exhaustive; it's
        // never actually reached with this message.
        Message::Profiles(profiles::Message::OpenEmbedded(tool, label)) => {
            let profile = state.profiles.profiles.iter().find(|p| p.tool == tool && p.label == label).cloned();
            match profile {
                Some(profile) => {
                    let env = crate::launch::launch_env(tool, &profile, &state.providers.providers);
                    let title = format!("{} · {}", profile.label, tool);
                    match embedded_terminal::open_tab(&mut state.terminal, title, None, env, Some(tool.as_str().to_string())) {
                        Ok(()) => state.screen = Screen::Terminal,
                        Err(e) => embedded_terminal::note_error(&mut state.terminal, format!("打开内嵌终端失败: {e}")),
                    }
                }
                None => {
                    embedded_terminal::note_error(&mut state.terminal, format!("本机没有 {tool} Profile '{label}'"))
                }
            }
            Task::none()
        }
        Message::Profiles(inner) => {
            profiles::update(&mut state.profiles, inner, &state.providers.providers).map(Message::Profiles)
        }
        Message::Providers(inner) => providers::update(&mut state.providers, inner).map(Message::Providers),
        // Same interception pattern as `OpenEmbedded` above, reusing
        // `projects::resumable`/`record_tool`/`resume_command` so the
        // eligibility rules and command line stay identical to the
        // external-window resume path.
        Message::Projects(projects::Message::ResumeEmbedded(path)) => {
            let record = state.projects.records.iter().find(|r| r.path == path).cloned();
            let Some(record) = record else {
                return Task::none();
            };
            if let Err(e) = projects::resumable(&record, &state.profiles.profiles) {
                embedded_terminal::note_error(&mut state.terminal, e);
                return Task::none();
            }
            let tool = projects::record_tool(&record);
            let profile = record
                .profile_label
                .as_ref()
                .and_then(|label| state.profiles.profiles.iter().find(|p| p.tool == tool && &p.label == label).cloned());
            let Some(profile) = profile else {
                // `resumable` above already checked this, but doesn't
                // hurt to fail closed rather than unwrap.
                return Task::none();
            };
            let env = crate::launch::launch_env(tool, &profile, &state.providers.providers);
            let command = projects::resume_command(tool, &record.last_session_id);
            let title = format!("{} · {} · {}", record.name, tool, profile.label);
            match embedded_terminal::open_tab(&mut state.terminal, title, Some(Path::new(&record.path).to_path_buf()), env, Some(command)) {
                Ok(()) => state.screen = Screen::Terminal,
                Err(e) => embedded_terminal::note_error(&mut state.terminal, format!("打开内嵌终端失败: {e}")),
            }
            Task::none()
        }
        Message::Projects(inner) => {
            projects::update(&mut state.projects, inner, &state.profiles.profiles, &state.providers.providers)
                .map(Message::Projects)
        }
        Message::Sessions(inner) => {
            sessions::update(&mut state.sessions, inner, &state.profiles.profiles, &state.providers.providers)
                .map(Message::Sessions)
        }
        Message::Skills(inner) => skills::update(&mut state.skills, inner, &state.profiles.profiles).map(Message::Skills),
        Message::Sync(inner) => sync::update(&mut state.sync, inner, &state.profiles.profiles).map(Message::Sync),
        Message::Terminal(inner) => embedded_terminal::update(&mut state.terminal, inner).map(Message::Terminal),
        Message::DismissWtBanner => {
            state.wt_banner_dismissed = true;
            Task::none()
        }
        Message::InstallWindowsTerminal => {
            // Only ever reachable from the button below -- never
            // triggered automatically just because `wt.exe` is missing.
            crate::task::perform(
                || crate::terminal::install_windows_terminal().map(|_child| ()).map_err(|e| e.to_string()),
                Message::WindowsTerminalInstallTriggered,
            )
        }
        Message::WindowsTerminalInstallTriggered(Ok(())) => {
            state.wt_install_status =
                Some("已通过 winget 触发安装，完成后重启 aam-gui 即可生效".to_string());
            Task::none()
        }
        Message::WindowsTerminalInstallTriggered(Err(e)) => {
            state.wt_install_status = Some(format!("触发安装失败（可能没有 winget）: {e}"));
            Task::none()
        }
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    let tabs = row![
        button(text("Profiles")).on_press(Message::SwitchScreen(Screen::Profiles)),
        button(text("Providers")).on_press(Message::SwitchScreen(Screen::Providers)),
        button(text("Projects")).on_press(Message::SwitchScreen(Screen::Projects)),
        button(text("Sessions")).on_press(Message::SwitchScreen(Screen::Sessions)),
        button(text("Skills")).on_press(Message::SwitchScreen(Screen::Skills)),
        button(text("Sync")).on_press(Message::SwitchScreen(Screen::Sync)),
        button(text("Terminal")).on_press(Message::SwitchScreen(Screen::Terminal)),
    ]
    .spacing(8);

    let body: Element<'_, Message> = match state.screen {
        Screen::Profiles => profiles::view(&state.profiles, &state.providers.providers).map(Message::Profiles),
        Screen::Providers => providers::view(&state.providers).map(Message::Providers),
        Screen::Projects => {
            projects::view(&state.projects, &state.profiles.profiles, &state.providers.providers).map(Message::Projects)
        }
        Screen::Sessions => sessions::view(&state.sessions, &state.profiles.profiles).map(Message::Sessions),
        Screen::Skills => skills::view(&state.skills).map(Message::Skills),
        Screen::Sync => sync::view(&state.sync, &state.profiles.profiles, &state.providers.providers).map(Message::Sync),
        Screen::Terminal => embedded_terminal::view(&state.terminal).map(Message::Terminal),
    };

    let mut content = column![tabs].spacing(8).width(Length::Fill).height(Length::Fill);
    if !state.wt_available && !state.wt_banner_dismissed {
        let mut banner = row![
            text("未检测到 Windows Terminal，打开的终端窗口会退回普通 PowerShell 窗口。"),
            button(text("现在安装")).on_press(Message::InstallWindowsTerminal),
            button(text("知道了")).on_press(Message::DismissWtBanner),
        ]
        .spacing(8);
        if let Some(status) = &state.wt_install_status {
            banner = banner.push(text(status.clone()));
        }
        content = content.push(banner);
    }
    content = content.push(body);

    container(content).padding(8).into()
}

/// First real use of `iced`'s `Subscription` mechanism in this app --
/// every prior screen only ever needed one-shot `Task`s (Phase 5 Round 1
/// plan). Only the Terminal screen has a continuous event stream to
/// listen to right now; later rounds (multi-tab) will aggregate more
/// than one terminal's subscription here via `Subscription::batch`.
pub fn subscription(state: &State) -> Subscription<Message> {
    embedded_terminal::subscription(&state.terminal).map(Message::Terminal)
}
