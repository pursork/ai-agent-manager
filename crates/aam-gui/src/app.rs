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

use iced::widget::{button, column, container, row, text};
use iced::{Element, Length, Task};

use crate::screens::{profiles, projects, providers, sessions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Profiles,
    Providers,
    Projects,
    Sessions,
}

pub struct State {
    screen: Screen,
    profiles: profiles::State,
    providers: providers::State,
    projects: projects::State,
    sessions: sessions::State,
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
        ]),
    )
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::SwitchScreen(screen) => {
            state.screen = screen;
            Task::none()
        }
        Message::Profiles(inner) => {
            profiles::update(&mut state.profiles, inner, &state.providers.providers).map(Message::Profiles)
        }
        Message::Providers(inner) => providers::update(&mut state.providers, inner).map(Message::Providers),
        Message::Projects(inner) => {
            projects::update(&mut state.projects, inner, &state.profiles.profiles, &state.providers.providers)
                .map(Message::Projects)
        }
        Message::Sessions(inner) => {
            sessions::update(&mut state.sessions, inner, &state.profiles.profiles, &state.providers.providers)
                .map(Message::Sessions)
        }
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
    ]
    .spacing(8);

    let body: Element<'_, Message> = match state.screen {
        Screen::Profiles => profiles::view(&state.profiles, &state.providers.providers).map(Message::Profiles),
        Screen::Providers => providers::view(&state.providers).map(Message::Providers),
        Screen::Projects => {
            projects::view(&state.projects, &state.profiles.profiles, &state.providers.providers).map(Message::Projects)
        }
        Screen::Sessions => sessions::view(&state.sessions, &state.profiles.profiles).map(Message::Sessions),
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
