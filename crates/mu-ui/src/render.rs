//! Layout and drawing: history pane, input band, two-line footer.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Position, Rect},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use crate::app::App;

/// Input band grows with content up to this many rows, then scrolls.
const INPUT_MAX_ROWS: usize = 4;
const PROMPT_GLYPH: &str = "›";
const PLACEHOLDER: &str = "Type a message…";

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    // A degenerate area (0x0 pty, minimized window, split-tab edge) can't
    // host the band's rules; bail out until a resize arrives instead of
    // panicking on underflowing rect math.
    if area.height < 3 || area.width < 2 {
        return;
    }
    // Two columns are reserved for the prompt glyph.
    let text_width = area.width.saturating_sub(2).max(1) as usize;
    let input_rows = app.input.rows(text_width).len().clamp(1, INPUT_MAX_ROWS) as u16;

    let chunks = Layout::vertical([
        Constraint::Min(1),                 // history
        Constraint::Length(input_rows + 2), // input band + top/bottom rules
        Constraint::Length(2),              // footer
    ])
    .split(area);

    draw_history(frame, app, chunks[0]);
    draw_input(frame, app, chunks[1], text_width);
    draw_footer(frame, app, chunks[2]);
}

/// Empty until rollout step 2 lands conversation items.
fn draw_history(_frame: &mut Frame, app: &App, _area: Rect) {
    debug_assert!(app.history.items().is_empty());
}

fn draw_input(frame: &mut Frame, app: &mut App, area: Rect, text_width: usize) {
    let block = Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(app.theme.border);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let columns = Layout::horizontal([Constraint::Length(2), Constraint::Min(1)]).split(inner);
    frame.render_widget(
        Paragraph::new(PROMPT_GLYPH).style(app.theme.prompt_glyph),
        columns[0],
    );

    app.input
        .ensure_cursor_visible(inner.height as usize, text_width);
    let scroll = app.input.scroll();

    if app.input.is_empty() {
        frame.render_widget(
            Paragraph::new(PLACEHOLDER).style(app.theme.placeholder),
            columns[1],
        );
    } else {
        let text = app.input.text();
        let lines: Vec<Line> = app
            .input
            .rows(text_width)
            .iter()
            .map(|&(start, end)| Line::from(text[start..end].to_string()))
            .collect();
        frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), columns[1]);
    }

    draw_scroll_indicators(frame, app, area, text_width);

    let (column, row) = app.input.cursor_pos(text_width);
    frame.set_cursor_position(Position::new(
        columns[1].x + column,
        columns[1].y + row.saturating_sub(scroll as u16),
    ));
}

/// While the input overflows its band, show how many rows are hidden
/// above/below on the top rule, right-anchored: one gap column, the
/// cluster (`↑ 2 ↓ 4`, `↑ 6`, or `↓ 6`), then the border's rule shows as
/// the `──` corner. Nothing once everything fits, so the rule never
/// overlaps the input text.
fn draw_scroll_indicators(frame: &mut Frame, app: &App, area: Rect, text_width: usize) {
    let total = app.input.rows(text_width).len();
    // Band area includes the two rule rows; the visible window is in
    // between.
    let height = area.height.saturating_sub(2) as usize;
    if total <= height || height == 0 {
        return;
    }
    // Belt-and-braces on top of ensure_cursor_visible's clamp: the count
    // math below must never underflow no matter how scroll got set.
    let scroll = app.input.scroll().min(total.saturating_sub(height));
    let above = scroll;
    let below = total - height - scroll;
    let cluster = match (above > 0, below > 0) {
        (true, true) => format!("↑{above:>2} ↓{below:>2}"),
        (true, false) => format!("↑{above:>2}"),
        (false, true) => format!("↓{below:>2}"),
        (false, false) => return,
    };
    // Right-anchored: a gap column, the cluster, another gap column, then
    // the border's own rule glyphs as the `──` corner. The gap cells erase
    // the border glyphs under them, floating the indicator between rule
    // runs.
    let mut width = cluster.chars().count() as u16 + 2;
    let x = area
        .right()
        .saturating_sub(width)
        .saturating_sub(2)
        .min(area.right());
    width = width.min(area.right().saturating_sub(x));
    if width == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(format!(" {cluster} ")).style(app.theme.scroll_indicator),
        Rect {
            x,
            y: area.y, // the top rule row
            width,
            height: 1,
        },
    );
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let lines = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

    let mut context = app.cwd_display.clone();
    if let Some(branch) = &app.branch {
        context.push_str(" (");
        context.push_str(branch);
        context.push(')');
    }
    frame.render_widget(Paragraph::new(context).style(app.theme.footer), lines[0]);

    let usage = format!(
        "↑{} ↓{}",
        format_tokens(app.usage.input_tokens),
        format_tokens(app.usage.output_tokens)
    );
    let columns = Layout::horizontal([
        Constraint::Length(usage.chars().count() as u16),
        Constraint::Min(1),
    ])
    .split(lines[1]);
    frame.render_widget(Paragraph::new(usage).style(app.theme.footer), columns[0]);
    frame.render_widget(
        Paragraph::new(app.config.model_display.clone())
            .style(app.theme.footer)
            .alignment(Alignment::Right),
        columns[1],
    );
}

fn format_tokens(count: u64) -> String {
    if count < 1000 {
        count.to_string()
    } else {
        format!("{:.1}k", count as f64 / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, AppConfig};
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    };

    fn up() -> KeyEvent {
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)
    }

    /// Draw the app and return every row's text, so tests can assert on
    /// visible content.
    fn draw_rows(app: &mut App) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                let mut row = String::new();
                for x in 0..buffer.area.width {
                    row.push_str(buffer[(x, y)].symbol());
                }
                row
            })
            .collect()
    }

    /// Split the input band into (top rule, text rows). The top rule may
    /// carry the scroll indicators; the bottom rule is the next row that is
    /// entirely `─`. Keeps the footer's "↑0 ↓0" usage out of assertions.
    fn split_band(rows: &[String]) -> (String, String) {
        let start = rows.iter().position(|row| row.starts_with('─')).unwrap_or(0);
        let end = rows[start + 1..]
            .iter()
            .position(|row| row.chars().all(|c| c == '─'))
            .map(|index| index + start + 1)
            .unwrap_or(rows.len());
        (rows[start].clone(), rows[start + 1..end].join("\n"))
    }

    /// Layout invariants: a gap column on each side of the cluster, the
    /// `──` corner rule, and nothing between the cluster's last glyph and
    /// the corner but the gap and rule.
    fn rule_to_corner_is_clean(rule: &str) -> bool {
        let chars: Vec<char> = rule.chars().collect();
        // The last non-rule cell is the trailing gap column.
        let Some(last) = chars.iter().rposition(|&c| c != '─') else {
            return false; // no indicator at all
        };
        let Some(arrow) = chars.iter().position(|&c| c == '↑' || c == '↓') else {
            return false;
        };
        chars[last] == ' '
            && chars[last + 1..].iter().all(|&c| c == '─')
            && chars.len() - last > 2
            && arrow > 0
            && chars[arrow - 1] == ' ' // leading gap
    }

    /// Hidden-row indicators track the scroll window, on the top rule,
    /// right-anchored: ` gap ↑ 6 ──`-style, with the `──` corner constant.
    #[test]
    fn scroll_indicators_reflect_hidden_rows() {
        let (mut app, _handle, _ui_events) = App::new(AppConfig::default());
        app.input.insert_str(
            "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10",
        );

        // Typed to the end: cursor on row 9 of 10, window shows rows 6-9,
        // so 6 rows are hidden above and none below.
        let rows = draw_rows(&mut app);
        let (rule, band) = split_band(&rows);
        assert!(rule.contains(" ↑ 6"), "expected ↑ 6 on the rule, got: {rule:?}");
        assert!(!rule.contains('↓'), "no ↓ on the rule, got: {rule:?}");
        assert!(!band.contains('↓'), "no ↓ in band expected");
        assert!(rule.ends_with("──"), "── corner expected, got: {rule:?}");
        assert!(rule_to_corner_is_clean(&rule), "layout must be clean, got: {rule:?}");

        // Walk the cursor up 7 rows → row 2, window rows 2-5: 2 hidden
        // above, 4 hidden below.
        for _ in 0..7 {
            app.input.handle_key(up(), 78);
        }
        let rows = draw_rows(&mut app);
        let (rule, _) = split_band(&rows);
        assert!(
            rule.contains(" ↑ 2 ↓ 4"),
            "expected ↑ 2 ↓ 4 on the rule, got: {rule:?}"
        );
        assert!(rule.ends_with("──"), "── corner expected, got: {rule:?}");
        assert!(rule_to_corner_is_clean(&rule), "layout must be clean, got: {rule:?}");

        // At the very top everything above fits in the window again.
        for _ in 0..2 {
            app.input.handle_key(up(), 78);
        }
        let rows = draw_rows(&mut app);
        let (rule, _) = split_band(&rows);
        assert!(rule.contains(" ↓ 6"), "expected ↓ 6 on the rule, got: {rule:?}");
        assert!(!rule.contains('↑'), "no ↑ on the rule, got: {rule:?}");
        assert!(rule.ends_with("──"), "── corner expected, got: {rule:?}");
        assert!(rule_to_corner_is_clean(&rule), "layout must be clean, got: {rule:?}");
    }

    /// A resize that reflows the input (narrow → wide) leaves `scroll`
    /// stale; the indicators must clamp instead of underflowing.
    #[test]
    fn scroll_clamps_after_reflow() {
        let (mut app, _handle, _ui_events) = App::new(AppConfig::default());
        let text = (0..10)
            .map(|i| format!("line{i} padded text 12345"))
            .collect::<Vec<_>>()
            .join("\n");
        app.input.insert_str(&text);

        // Walk to the bottom at a narrow width: each line wraps into ~2
        // rows, so scroll goes deep (~16). Then draw at 80 cols, where the
        // same text is only 10 rows — scroll must clamp, not underflow.
        app.input.ensure_cursor_visible(4, 12);
        assert!(app.input.scroll() > 10, "precondition: deep scroll");

        let rows = draw_rows(&mut app);
        let (rule, band) = split_band(&rows);
        assert!(rule.contains("↑ 6"), "expected ↑ 6 on the rule, got: {rule:?}");
        assert!(!rule.contains('↓'), "no ↓ on the rule, got: {rule:?}");
        assert!(!band.contains('↓'), "no ↓ in band expected");
    }

    /// Footer layout: `cwd (branch)` on line 1; usage on the left of line
    /// 2, model/provider right-aligned on the right.
    #[test]
    fn footer_usage_left_model_right() {
        let (mut app, _handle, _ui_events) = App::new(AppConfig {
            model_display: "(opencode-go) deepseek-v4-flash".to_string(),
            ..AppConfig::default()
        });
        let rows = draw_rows(&mut app);
        let line1 = &rows[rows.len() - 2];
        let line2 = &rows[rows.len() - 1];
        assert!(line1.contains(" ("), "branch parenthesized: {line1:?}");
        assert!(line1.trim_end().ends_with(')'), "line 1 ends with ): {line1:?}");
        assert!(line2.starts_with("↑0 ↓0"), "usage left: {line2:?}");
        assert!(
            line2.trim_end().ends_with("(opencode-go) deepseek-v4-flash"),
            "model right: {line2:?}"
        );
    }
}
