//! AuxiNux colour theme — black background, white text, ORANGE highlight (logo
//! colour e8590c), white text when highlighted. Matches the look the bash
//! installer reproduces via NEWT_COLORS + the console palette reprogramming.

use ratatui::style::{Color, Modifier, Style};

/// AuxiNux logo orange (palette index 3 in the bash installer).
pub const ORANGE: Color = Color::Rgb(0xe8, 0x59, 0x0c);

/// Default text: white on black.
pub fn base() -> Style {
    Style::default().fg(Color::White).bg(Color::Black)
}

/// Focused / selected element: white on orange, bold.
pub fn highlight() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(ORANGE)
        .add_modifier(Modifier::BOLD)
}

/// Section + window titles: orange, bold.
pub fn title() -> Style {
    Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)
}

/// Secondary / hint text.
pub fn dim() -> Style {
    Style::default().fg(Color::Gray).bg(Color::Black)
}

/// Warnings.
pub fn warn() -> Style {
    Style::default()
        .fg(Color::Rgb(0xff, 0xb3, 0x4d))
        .bg(Color::Black)
        .add_modifier(Modifier::BOLD)
}

/// Errors.
pub fn error() -> Style {
    Style::default()
        .fg(Color::Rgb(0xff, 0x5c, 0x5c))
        .bg(Color::Black)
        .add_modifier(Modifier::BOLD)
}

/// Success / done.
pub fn ok() -> Style {
    Style::default()
        .fg(Color::Rgb(0x4a, 0xd6, 0x6e))
        .bg(Color::Black)
        .add_modifier(Modifier::BOLD)
}
