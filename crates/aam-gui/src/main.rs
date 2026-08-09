//! `aam-gui` — the `iced` GUI shell (`docs/06-gui-terminal-shell.md`).
//!
//! Phase 4 Round 1: Profile + Provider management, wrapping the same
//! `aam-switcher` public API `aam-cli` already uses. No business logic
//! lives here (§6.2) -- see `app.rs` and `screens/*`.

mod app;
mod screens;
mod task;
mod terminal;

fn main() -> iced::Result {
    iced::application(app::new, app::update, app::view)
        .title("ai-agent-manager")
        .run()
}
