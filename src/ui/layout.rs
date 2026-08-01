use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Split the terminal into areas.
pub fn create_layout(area: Rect) -> (Rect, Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),    // Message area (flexible)
            Constraint::Length(3), // Input area (fixed)
            Constraint::Length(1), // Status bar (fixed)
        ])
        .split(area);

    (chunks[0], chunks[1], chunks[2])
}
