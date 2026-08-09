//! Layout and drawing: history pane, input band, two-line footer.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Position, Rect},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
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

/// The history pane: every item is wrapped with ratatui's native `Paragraph`
/// wrapping, heights cached per item and invalidated only on width change.
/// Scroll is a flattened-line offset into the sum of those heights; only the
/// items intersecting the visible window are drawn, partially visible ones
/// positioned with `Paragraph::scroll`.
fn draw_history(frame: &mut Frame, app: &mut App, area: Rect) {
    app.history_width = area.width;
    app.history_height = area.height;
    let viewport = area.height as usize;
    if viewport == 0 || area.width == 0 {
        return;
    }

    let total = app.history.total_height(area.width);
    let max_offset = total.saturating_sub(viewport);
    if app.sticky_bottom {
        app.scroll_offset = max_offset;
    } else {
        app.scroll_offset = app.scroll_offset.min(max_offset);
    }
    let top = app.scroll_offset;
    let bottom = top + viewport;

    let items = app.history.items().len();
    let mut flat = 0usize;
    for index in 0..items {
        let height = app.history.item_height(index, area.width);
        let item_top = flat;
        flat += height;
        if flat <= top || item_top >= bottom {
            continue;
        }
        let vis_top = item_top.max(top);
        let vis_bottom = flat.min(bottom);
        let local = vis_top - item_top;
        let lines = app.history.items()[index].styled_lines(&app.theme);
        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((local as u16, 0));
        frame.render_widget(
            paragraph,
            Rect {
                x: area.x,
                y: area.y + (vis_top - top) as u16,
                width: area.width,
                height: (vis_bottom - vis_top) as u16,
            },
        );
    }
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
        buffer::Buffer,
        crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    };

    fn up() -> KeyEvent {
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)
    }

    /// Every row of a rendered buffer as text.
    fn rows_of(buffer: &Buffer) -> Vec<String> {
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

    /// Draw the app and return every row's text, so tests can assert on
    /// visible content.
    fn draw_rows(app: &mut App) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        rows_of(terminal.backend().buffer())
    }

    /// The rows of the history pane only (up to the input band's top rule).
    fn history_rows(rows: &[String]) -> &[String] {
        let end = rows
            .iter()
            .position(|row| row.starts_with('─'))
            .unwrap_or(rows.len());
        &rows[..end]
    }

    /// First cell of `needle` in the buffer, if visible.
    fn locate(buffer: &Buffer, needle: &str) -> Option<(u16, u16)> {
        for (y, row) in rows_of(buffer).iter().enumerate() {
            if let Some(offset) = row.find(needle) {
                return Some((offset as u16, y as u16));
            }
        }
        None
    }

    /// Split the input band into (top rule, text rows). The top rule may
    /// carry the scroll indicators; the bottom rule is the next row that is
    /// entirely `─`. Keeps the footer's "↑0 ↓0" usage out of assertions.
    fn split_band(rows: &[String]) -> (String, String) {
        let start = rows
            .iter()
            .position(|row| row.starts_with('─'))
            .unwrap_or(0);
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
        app.input
            .insert_str("line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10");

        // Typed to the end: cursor on row 9 of 10, window shows rows 6-9,
        // so 6 rows are hidden above and none below.
        let rows = draw_rows(&mut app);
        let (rule, band) = split_band(&rows);
        assert!(
            rule.contains(" ↑ 6"),
            "expected ↑ 6 on the rule, got: {rule:?}"
        );
        assert!(!rule.contains('↓'), "no ↓ on the rule, got: {rule:?}");
        assert!(!band.contains('↓'), "no ↓ in band expected");
        assert!(rule.ends_with("──"), "── corner expected, got: {rule:?}");
        assert!(
            rule_to_corner_is_clean(&rule),
            "layout must be clean, got: {rule:?}"
        );

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
        assert!(
            rule_to_corner_is_clean(&rule),
            "layout must be clean, got: {rule:?}"
        );

        // At the very top everything above fits in the window again.
        for _ in 0..2 {
            app.input.handle_key(up(), 78);
        }
        let rows = draw_rows(&mut app);
        let (rule, _) = split_band(&rows);
        assert!(
            rule.contains(" ↓ 6"),
            "expected ↓ 6 on the rule, got: {rule:?}"
        );
        assert!(!rule.contains('↑'), "no ↑ on the rule, got: {rule:?}");
        assert!(rule.ends_with("──"), "── corner expected, got: {rule:?}");
        assert!(
            rule_to_corner_is_clean(&rule),
            "layout must be clean, got: {rule:?}"
        );
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
        assert!(
            rule.contains("↑ 6"),
            "expected ↑ 6 on the rule, got: {rule:?}"
        );
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
        assert!(
            line1.trim_end().ends_with(')'),
            "line 1 ends with ): {line1:?}"
        );
        assert!(line2.starts_with("↑0 ↓0"), "usage left: {line2:?}");
        assert!(
            line2
                .trim_end()
                .ends_with("(opencode-go) deepseek-v4-flash"),
            "model right: {line2:?}"
        );
    }

    /// A submitted prompt and a delivered message both appear in the history
    /// pane, edge-to-edge (no borders, no padding).
    #[test]
    fn prompt_and_message_render_edge_to_edge() {
        let (mut app, _handle, _ui_events) = App::new(AppConfig::default());
        app.history.user_prompt("user asks a thing".to_string());
        app.history.message_start();
        app.history.message_delta("assistant answers something");
        app.history.message_end();

        let rows = draw_rows(&mut app);
        let joined = history_rows(&rows).join("\n");
        assert!(
            joined.contains("user asks a thing"),
            "prompt visible: {joined:?}"
        );
        assert!(
            joined.contains("assistant answers something"),
            "message visible: {joined:?}"
        );
        assert!(
            joined.lines().all(|line| !line.starts_with('│')),
            "no side borders anywhere: {joined:?}"
        );
    }

    /// Lines inside a ``` fence render dimmed; plain lines do not.
    #[test]
    fn code_fence_gets_dim_background() {
        let (mut app, _handle, _ui_events) = App::new(AppConfig::default());
        app.history.message_start();
        app.history
            .message_delta("intro line\n```rust\nlet x = 1;\n```\noutro line");
        app.history.message_end();

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let fenced_bg = app
            .theme
            .code_fence
            .bg
            .expect("code_fence has a background");

        let (fx, fy) = locate(buffer, "let x = 1;").expect("fence content visible");
        assert_eq!(buffer[(fx, fy)].bg, fenced_bg, "fenced line is dimmed");
        let (px, py) = locate(buffer, "intro line").expect("plain content visible");
        assert_ne!(buffer[(px, py)].bg, fenced_bg, "plain line is not dimmed");
    }

    /// The native-wrapping experiment: heights come from `Paragraph::line_count`
    /// and re-wrap only when the width changes.
    #[test]
    fn resize_rewraps_item_heights() {
        let (mut app, _handle, _ui_events) = App::new(AppConfig::default());
        let text = vec!["alpha"; 30].join(" ");
        app.history.message_start();
        app.history.message_delta(&text);
        app.history.message_end();

        let wide = app.history.item_height(0, 80);
        let narrow = app.history.item_height(0, 40);
        assert!(narrow > wide, "narrower width must produce more rows");
        assert_eq!(
            app.history.item_height(0, 80),
            wide,
            "cache stable at a width"
        );
    }

    /// A partially visible item is drawn at its correct scroll position:
    /// with the window starting inside the message, its earlier lines are
    /// hidden and the visible ones start mid-item.
    #[test]
    fn partial_item_scrolls_mid_message() {
        let (mut app, _handle, _ui_events) = App::new(AppConfig::default());
        app.history.user_prompt("PROMPT".to_string());
        app.history.message_start();
        app.history
            .message_delta("line one\nline two\nline three\nline four\nline five");
        app.history.message_end();
        app.history_width = 40;
        app.history_height = 4;
        // Each item owns a trailing separator row, so PROMPT spans rows 0–1
        // and the message spans 2–7; offset 3 starts the window inside it.
        app.sticky_bottom = false;
        app.scroll_offset = 3;

        let mut terminal = Terminal::new(TestBackend::new(40, 4)).unwrap();
        terminal
            .draw(|frame| draw_history(frame, &mut app, Rect::new(0, 0, 40, 4)))
            .unwrap();
        let rows = rows_of(terminal.backend().buffer());
        assert_eq!(
            rows[0].trim_end(),
            "line two",
            "window starts mid-item: {rows:?}"
        );
        let joined = rows.join("\n");
        assert!(
            !joined.contains("line one"),
            "earlier lines hidden: {joined:?}"
        );
        assert!(
            !joined.contains("PROMPT"),
            "previous item hidden: {joined:?}"
        );
    }

    /// While sticky, drawing pins the window to the newest content even as it
    /// grows; a scrolled-up window stays put.
    #[test]
    fn draw_pins_to_bottom_when_sticky() {
        let (mut app, _handle, _ui_events) = App::new(AppConfig::default());
        app.history.message_start();
        app.history.message_delta(&"a ".repeat(300));
        app.history.message_end();
        app.history.message_start();
        app.history.message_delta(&"b ".repeat(300));
        app.history.message_end();
        app.scroll_offset = 0;
        app.sticky_bottom = true;

        draw_rows(&mut app);
        let expected = app
            .history
            .total_height(app.history_width)
            .saturating_sub(app.history_height as usize);
        assert_eq!(
            app.scroll_offset, expected,
            "sticky draw pins to the bottom"
        );

        // Not sticky: a draw must not move the window even as content grows.
        app.sticky_bottom = false;
        app.scroll_offset = expected.saturating_sub(1);
        let keep = app.scroll_offset;
        draw_rows(&mut app);
        assert_eq!(app.scroll_offset, keep, "scrolled-up draw stays put");
    }
}
