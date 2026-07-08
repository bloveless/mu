//! Central place for all colors/styles used across the UI.
//!
//! Keeping every `Style` here means re-skinning the app (e.g. for a light
//! terminal theme, or to match a brand palette) is a one-file change instead
//! of a grep-and-replace across every widget.

use ratatui::style::{Color, Modifier, Style};

pub struct Theme;

impl Theme {
    // ---- History: user turn ----
    pub fn user_tag() -> Style {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    }
    pub fn user_text() -> Style {
        Style::default().fg(Color::Cyan)
    }

    // ---- History: agent turn ----
    pub fn agent_tag() -> Style {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    }
    pub fn agent_text() -> Style {
        Style::default().fg(Color::White)
    }

    // ---- History: thinking block ----
    pub fn thinking_tag() -> Style {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC)
    }
    pub fn thinking_text() -> Style {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC)
    }

    // ---- History: tool call bubble ----
    pub fn tool_badge_running() -> Style {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    }
    pub fn tool_badge_success() -> Style {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    }
    pub fn tool_badge_failed() -> Style {
        Style::default()
            .fg(Color::White)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    }
    pub fn tool_args() -> Style {
        Style::default().fg(Color::Yellow)
    }
    pub fn tool_output() -> Style {
        Style::default().fg(Color::Gray)
    }
    pub fn tool_output_bar() -> Style {
        Style::default().fg(Color::DarkGray)
    }

    // ---- History: system notes (errors, cancellation) ----
    pub fn system_error() -> Style {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    }
    pub fn system_info() -> Style {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    }

    // ---- Chrome ----
    pub fn border_idle() -> Style {
        Style::default().fg(Color::DarkGray)
    }
    pub fn border_active() -> Style {
        Style::default().fg(Color::Blue)
    }
    pub fn status_bar() -> Style {
        Style::default().fg(Color::Black).bg(Color::Gray)
    }
    pub fn status_bar_accent() -> Style {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Gray)
            .add_modifier(Modifier::BOLD)
    }
    pub fn scroll_indicator() -> Style {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    }

    // ---- Autocomplete popup ----
    pub fn popup_border() -> Style {
        Style::default().fg(Color::Magenta)
    }
    pub fn popup_selected() -> Style {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Magenta)
            .add_modifier(Modifier::BOLD)
    }

    // ---- Modal ----
    pub fn modal_border() -> Style {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    }
}
