//! Centralized styles. Hardcoded dark-terminal defaults for now; the struct
//! is the configuration seam later.

use ratatui::style::{Color, Modifier, Style};

// Several fields are consumed by later rollout steps (thinking, tool calls,
// errors).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Theme {
    pub prompt_glyph: Style,
    pub placeholder: Style,
    pub border: Style,
    pub footer: Style,
    pub scroll_indicator: Style,
    pub thinking: Style,
    pub user_prompt: Style,
    pub message: Style,
    pub code_fence: Style,
    pub tool_running: Style,
    pub tool_success: Style,
    pub tool_failure: Style,
    pub error: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            prompt_glyph: Style::default().fg(Color::Cyan),
            placeholder: Style::default().fg(Color::DarkGray),
            border: Style::default().fg(Color::DarkGray),
            footer: Style::default().fg(Color::DarkGray),
            scroll_indicator: Style::default().fg(Color::DarkGray),
            thinking: Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
            user_prompt: Style::default().fg(Color::Cyan),
            message: Style::default(),
            code_fence: Style::default()
                .fg(Color::DarkGray)
                .bg(Color::Rgb(0x20, 0x20, 0x20)),
            tool_running: Style::default().fg(Color::Yellow),
            tool_success: Style::default().fg(Color::Green),
            tool_failure: Style::default().fg(Color::Red),
            error: Style::default().fg(Color::Red),
        }
    }
}
