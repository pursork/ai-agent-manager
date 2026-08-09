//! Embedded multi-tab terminal (Phase 5 Round 2, building on Round 1's
//! single fixed tab that proved `iced_term` renders/accepts input at
//! all -- `docs/06-gui-terminal-shell.md` §§6.11-6.12).
//!
//! `open_tab` is the entry point other screens use (via `app.rs`) to
//! open a tab already configured for a Profile/project resume -- this
//! module itself only owns the tab bar and the bare "+ 新终端" button;
//! it has no idea what a Profile or a Provider is (§6.2's "no business
//! logic" boundary).
//!
//! Deliberately doesn't touch `crate::terminal` (Phase 4's external-
//! window launch primitive, still available side-by-side) -- but does
//! share its `powershell_args` so both paths build the same command
//! line the same way.

use std::path::PathBuf;

use iced::widget::{column, container, row, text};
use iced::{Element, Length, Subscription, Task};
use iced_term::settings::{BackendSettings, Settings};
use iced_term::{Command as TermCommand, Terminal};

use crate::style::{primary_button, secondary_button, SPACING_LG, SPACING_SM};

struct Tab {
    id: u64,
    title: String,
    term: Terminal,
}

pub struct State {
    tabs: Vec<Tab>,
    active: Option<u64>,
    next_id: u64,
    error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        let mut state = Self {
            tabs: Vec::new(),
            active: None,
            next_id: 0,
            error: None,
        };
        // Round 1's behavior continues: never start on a completely
        // empty screen -- one plain shell tab always exists up front.
        if let Err(e) = open_tab(&mut state, "本地终端".to_string(), None, Vec::new(), None) {
            state.error = Some(format!("打开默认终端失败: {e}"));
        }
        state
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    TermEvent(iced_term::Event),
    NewTab,
    CloseTab(u64),
    SelectTab(u64),
}

/// Opens a new tab and makes it active. `command` is `None` for a bare
/// interactive shell (the "+ 新终端" button), `Some(cmd)` to run `cmd`
/// immediately and stay open afterwards (`-NoExit`, same as
/// `crate::terminal::open_terminal`'s external-window path) -- used by
/// `app.rs` when a Profile/project launch asks for an embedded tab
/// instead of an external window.
///
/// `pub` for `app.rs` to call directly; this is *not* wired to any
/// `Message` variant of its own, since the caller already knows
/// everything (title/cwd/env/command) up front -- there's no user
/// interaction within this screen that triggers it.
pub fn open_tab(
    state: &mut State,
    title: String,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    command: Option<String>,
) -> Result<(), String> {
    let id = state.next_id;
    state.next_id += 1;
    let settings = Settings {
        backend: BackendSettings {
            // Windows' default is `wsl.exe` (Round 1 finding) -- this
            // project is PowerShell-only.
            program: "powershell.exe".to_string(),
            args: command.as_deref().map(crate::terminal::powershell_args).unwrap_or_default(),
            env: env.into_iter().collect(),
            working_directory: cwd,
        },
        ..Default::default()
    };
    let term = Terminal::new(id, settings).map_err(|e| e.to_string())?;
    state.tabs.push(Tab { id, title, term });
    state.active = Some(id);
    state.error = None;
    Ok(())
}

/// Surfaces a message on this screen even though the caller (`app.rs`,
/// e.g. after a failed `open_tab` triggered from Profiles/Projects)
/// isn't this screen's own `update` loop.
pub fn note_error(state: &mut State, msg: String) {
    state.error = Some(msg);
}

fn close_tab(state: &mut State, id: u64) {
    state.tabs.retain(|t| t.id != id);
    if state.active == Some(id) {
        // Falls back to whatever tab is now last, if any -- not
        // "the previous tab" specifically; with only single-digit tab
        // counts expected in practice, simplicity wins over precise
        // MRU tracking here.
        state.active = state.tabs.last().map(|t| t.id);
    }
}

pub fn subscription(state: &State) -> Subscription<Message> {
    // Background tabs keep receiving events too -- switching away from
    // a tab must not stall the output piling up inside it.
    Subscription::batch(state.tabs.iter().map(|t| t.term.subscription().map(Message::TermEvent)))
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::TermEvent(iced_term::Event::BackendCall(id, cmd)) => {
            if let Some(tab) = state.tabs.iter_mut().find(|t| t.id == id) {
                let action = tab.term.handle(TermCommand::ProxyToBackend(cmd));
                if matches!(action, iced_term::actions::Action::Shutdown) {
                    close_tab(state, id);
                }
            }
            Task::none()
        }
        Message::NewTab => {
            let label = format!("终端 {}", state.next_id + 1);
            if let Err(e) = open_tab(state, label, None, Vec::new(), None) {
                state.error = Some(format!("打开终端失败: {e}"));
            }
            Task::none()
        }
        Message::CloseTab(id) => {
            close_tab(state, id);
            Task::none()
        }
        Message::SelectTab(id) => {
            if state.tabs.iter().any(|t| t.id == id) {
                state.active = Some(id);
            }
            Task::none()
        }
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    let mut tab_bar = row![].spacing(SPACING_SM);
    for tab in &state.tabs {
        let is_active = state.active == Some(tab.id);
        let label = if is_active { format!("▶ {}", tab.title) } else { tab.title.clone() };
        tab_bar = tab_bar.push(
            row![
                secondary_button(label, Some(Message::SelectTab(tab.id))),
                secondary_button("×", Some(Message::CloseTab(tab.id))),
            ]
            .spacing(2.0),
        );
    }
    tab_bar = tab_bar.push(primary_button("+ 新终端", Some(Message::NewTab)));

    let active_tab = state.active.and_then(|id| state.tabs.iter().find(|t| t.id == id));
    let body: Element<'_, Message> = match active_tab {
        Some(tab) => iced_term::TerminalView::show(&tab.term).map(Message::TermEvent),
        None => text("点上面「+ 新终端」，或去 Profiles/Projects 页接续").into(),
    };

    // Shown regardless of whether there's an active tab -- a failed
    // `open_tab` triggered from Profiles/Projects (via `note_error`)
    // needs to stay visible even when this screen already has an
    // unrelated tab open (the common case, since one always exists by
    // default), not just in the "no tabs at all" fallback above.
    let mut content = column![text("Terminal").size(24), tab_bar];
    if let Some(error) = &state.error {
        content = content.push(text(error.clone()));
    }
    content = content.push(body);

    container(content.spacing(SPACING_LG))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(SPACING_LG)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // These really do spawn `powershell.exe` PTYs (per the plan's own
    // escape hatch: "构造 `BackendSettings`" alone isn't interesting
    // enough to test in isolation -- the actual logic worth verifying,
    // id assignment and active-tab fallback on close, only exists once
    // real `Tab`s exist). Same category as `aam-skills`'s real-`git`
    // tests: a real, always-available local program, no network/
    // credentials involved.

    fn empty_state() -> State {
        State { tabs: Vec::new(), active: None, next_id: 0, error: None }
    }

    #[test]
    fn open_tab_assigns_increasing_ids_and_activates_the_new_tab() {
        let mut state = empty_state();
        open_tab(&mut state, "a".to_string(), None, Vec::new(), None).unwrap();
        open_tab(&mut state, "b".to_string(), None, Vec::new(), None).unwrap();
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.tabs[0].id, 0);
        assert_eq!(state.tabs[1].id, 1);
        assert_eq!(state.active, Some(1));
    }

    #[test]
    fn close_tab_falls_back_to_the_last_remaining_tab_then_to_none() {
        let mut state = empty_state();
        open_tab(&mut state, "a".to_string(), None, Vec::new(), None).unwrap();
        open_tab(&mut state, "b".to_string(), None, Vec::new(), None).unwrap();
        let first_id = state.tabs[0].id;
        let second_id = state.tabs[1].id;

        close_tab(&mut state, second_id); // closing the active tab
        assert_eq!(state.active, Some(first_id));

        close_tab(&mut state, first_id); // closing the only remaining tab
        assert_eq!(state.active, None);
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn close_tab_leaves_active_selection_alone_for_a_background_tab() {
        let mut state = empty_state();
        open_tab(&mut state, "a".to_string(), None, Vec::new(), None).unwrap();
        open_tab(&mut state, "b".to_string(), None, Vec::new(), None).unwrap();
        let first_id = state.tabs[0].id;
        let active_before = state.active;

        close_tab(&mut state, first_id); // not the active one
        assert_eq!(state.active, active_before);
    }
}
