//! Phase 5 Round 1: a single, fixed, no-business-logic embedded terminal
//! tab -- proves `iced_term` actually renders and accepts input inside
//! this app before any tab management or "resume a project into an
//! embedded tab" wiring gets built on top of it (`docs/06-gui-terminal-
//! shell.md` §6.11's plan: derisk the new technology first).
//!
//! Deliberately doesn't touch `crate::terminal` (Phase 4's external-
//! window launch primitive) -- that logic stays as-is; this is a wholly
//! separate rendering path.

use iced::widget::{column, container, text};
use iced::{Element, Length, Subscription, Task};
use iced_term::settings::{BackendSettings, Settings};
use iced_term::{Command as TermCommand, Terminal};

use crate::style::{primary_button, SPACING_LG, SPACING_MD};

const TERM_ID: u64 = 0;

pub struct State {
    term: Option<Terminal>,
    exited: bool,
    error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        // A missing `powershell.exe` or PTY-creation failure shouldn't
        // take the whole app down, just this one screen -- shown via
        // `error` and recoverable with the "重新打开" button, not a panic.
        match new_terminal() {
            Ok(term) => Self { term: Some(term), exited: false, error: None },
            Err(e) => Self { term: None, exited: false, error: Some(format!("打开终端失败: {e}")) },
        }
    }
}

/// Windows' default `program` for `iced_term` is `wsl.exe` (confirmed
/// from its `settings.rs` source before writing this) -- wrong for a
/// project that's entirely PowerShell-based (`terminal.rs`'s same
/// choice). `args`/`env`/`working_directory` are left at their defaults
/// this round -- wiring in `launch_env`/a project's cwd is Round 2's job,
/// this round only proves the rendering/input path works at all.
fn new_terminal() -> std::io::Result<Terminal> {
    let settings = Settings {
        backend: BackendSettings {
            program: "powershell.exe".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    Terminal::new(TERM_ID, settings)
}

#[derive(Debug, Clone)]
pub enum Message {
    TermEvent(iced_term::Event),
    Reopen,
}

pub fn subscription(state: &State) -> Subscription<Message> {
    match &state.term {
        Some(term) => term.subscription().map(Message::TermEvent),
        None => Subscription::none(),
    }
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::TermEvent(iced_term::Event::BackendCall(_id, cmd)) => {
            if let Some(term) = &mut state.term {
                let action = term.handle(TermCommand::ProxyToBackend(cmd));
                if matches!(action, iced_term::actions::Action::Shutdown) {
                    state.exited = true;
                    state.term = None;
                }
            }
            Task::none()
        }
        Message::Reopen => {
            state.exited = false;
            state.error = None;
            match new_terminal() {
                Ok(term) => state.term = Some(term),
                Err(e) => state.error = Some(format!("重新打开终端失败: {e}")),
            }
            Task::none()
        }
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    let body: Element<'_, Message> = if let Some(term) = &state.term {
        iced_term::TerminalView::show(term).map(Message::TermEvent)
    } else {
        let reason = if state.exited {
            "终端进程已退出"
        } else {
            state.error.as_deref().unwrap_or("终端还没打开")
        };
        column![text(reason), primary_button("重新打开", Some(Message::Reopen))]
            .spacing(SPACING_MD)
            .into()
    };

    container(column![text("Terminal（Phase 5 Round 1 验证）").size(24), body].spacing(SPACING_LG))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(SPACING_LG)
        .into()
}
