use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use super::app::AppState;

pub fn render_input(frame: &mut Frame, area: Rect, state: &AppState) {
    let input = Paragraph::new(state.input.as_str()).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Input (Enter to send, Ctrl+C to quit) ")
            .border_style(Style::default().fg(if state.loading {
                Color::Gray
            } else {
                Color::Cyan
            })),
    );

    frame.render_widget(input, area);

    // Position the cursor
    if !state.loading {
        frame.set_cursor_position((
            area.x + state.cursor as u16 + 1, // +1 for border
            area.y + 1,                       // +1 for border
        ));
    }
}
