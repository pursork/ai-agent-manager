//! Shared visual conventions so every screen reads as the same app,
//! not four independently-styled ones -- consistency is part of "user
//! friendly", not a separate nice-to-have on top of it (Phase 4 Round 2
//! plan's design principles).

use iced::widget::{button, text, Button};

pub const SPACING_SM: f32 = 4.0;
pub const SPACING_MD: f32 = 8.0;
pub const SPACING_LG: f32 = 16.0;

/// The one prominent action on a row/card (e.g. "接续", "创建", "保存") --
/// every screen has exactly one of these per row/form, so it's visually
/// obvious what the primary next step is (design principle #2).
pub fn primary_button<'a, Message: Clone + 'a>(label: &'a str, on_press: Option<Message>) -> Button<'a, Message> {
    button(text(label)).on_press_maybe(on_press).padding([8, 16]).style(button::primary)
}

/// A lower-emphasis action (e.g. "详情", "验证", "打开终端", "关联") --
/// still clickable, but doesn't compete visually with the row's primary
/// action.
pub fn secondary_button<'a, Message: Clone + 'a>(label: &'a str, on_press: Option<Message>) -> Button<'a, Message> {
    button(text(label)).on_press_maybe(on_press).padding([6, 12]).style(button::secondary)
}
