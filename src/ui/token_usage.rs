use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::app::AppState;

pub fn render_status_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let status = if let Some(ref usage) = state.token_usage {
        let color = if usage.percentage >= usage.threshold * 100.0 {
            Color::Red
        } else if usage.percentage >= usage.threshold * 75.0 {
            Color::Yellow
        } else {
            Color::Green
        };

        Line::from(vec![
            Span::raw(" Tokens: "),
            Span::styled(
                format!("{:.1}%", usage.percentage),
                Style::default().fg(color),
            ),
            Span::styled(
                format!(" ({}/{})", usage.used, usage.limit),
                Style::default().fg(Color::Gray),
            ),
        ])
    } else {
        Line::from(Span::styled(" Ready", Style::default().fg(Color::Green)))
    };

    frame.render_widget(Paragraph::new(status), area)
}
