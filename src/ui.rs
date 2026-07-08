//! # [Ratatui] `Scrollbar` example
//!
//! The latest version of this example is available in the [widget examples] folder in the
//! repository.
//!
//! Please note that the examples are designed to be run against the `main` branch of the Github
//! repository. This means that you may not be able to compile with the latest release version on
//! crates.io, or the one that you have installed locally.
//!
//! See the [examples readme] for more information on finding examples that match the version of the
//! library you are using.
//!
//! [Ratatui]: https://github.com/ratatui/ratatui
//! [widget examples]: https://github.com/ratatui/ratatui/blob/main/ratatui-widgets/examples
//! [examples readme]: https://github.com/ratatui/ratatui/blob/main/examples/README.md

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};

use crate::theme::Theme;
use crate::wrap::wrap;

/// Render the UI with vertical/horizontal scrollbars.
pub fn render(frame: &mut Frame, vertical: &mut ScrollbarState) {
    let [main, prompt] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(5)]).areas(frame.area());
    let [content, scrollbar] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(1)])
        .spacing(2)
        .areas(main);

    render_content(frame, content, vertical);
    render_vertical_scrollbar(frame, scrollbar, vertical);
    render_prompt(frame, prompt, "Hello");
}

/// Render a vertical scrollbar on the right side of the area.
pub fn render_vertical_scrollbar(frame: &mut Frame, area: Rect, vertical: &mut ScrollbarState) {
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        vertical,
    );
}

/// Render some content.
fn render_content(frame: &mut Frame, area: Rect, vertical: &ScrollbarState) {
    let mut content = vec![Line::from(Span::styled("▌ USER ", Theme::user_tag()))];
    for l in wrap(
        "Lorem ipsum dolor sit amet, consectetur adipiscing elit."
            .repeat(10)
            .as_str(),
        area.width.saturating_sub(2),
    ) {
        content.push(Line::from(Span::styled(
            format!("  {l}"),
            Theme::user_text(),
        )));
    }
    content.push(Line::default());
    content.push(Line::from(Span::styled("▌ ASSISTANT ", Theme::agent_tag())));
    for l in wrap(
        "Lorem ipsum dolor sit amet, consectetur adipiscing elit."
            .repeat(10)
            .as_str(),
        area.width.saturating_sub(2),
    ) {
        content.push(Line::from(Span::styled(
            format!("  {l}"),
            Theme::thinking_text(),
        )));
    }
    content.push(Line::default());
    for l in wrap(
        "Lorem ipsum dolor sit amet, consectetur adipiscing elit."
            .repeat(10)
            .as_str(),
        area.width.saturating_sub(2),
    ) {
        content.push(Line::from(Span::styled(
            format!("  {l}"),
            Theme::agent_text(),
        )));
    }
    content.push(Line::default());
    content.push(Line::from(Span::styled(
        " BASH ",
        Theme::tool_badge_running(),
    )));
    content.push(Line::from(Span::styled(
        " BASH ",
        Theme::tool_badge_failed(),
    )));
    content.push(Line::from(Span::styled(
        " BASH ",
        Theme::tool_badge_success(),
    )));
    for l in wrap(
        "Lorem ipsum dolor sit amet, consectetur adipiscing elit."
            .repeat(10)
            .as_str(),
        area.width.saturating_sub(2),
    ) {
        content.push(Line::from(Span::styled(
            format!("  {l}"),
            Theme::tool_output(),
        )));
    }
    content.push(Line::default());
    content.push(Line::from_iter([
        "Vertical: ".bold(),
        vertical.get_position().to_string().yellow(),
    ]));
    content.push(Line::default());
    frame.render_widget(
        Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .scroll((vertical.get_position() as u16, 0)),
        area,
    );
}

fn render_prompt(frame: &mut Frame, area: Rect, input: &str) {
    let input = Paragraph::new(input).block(Block::bordered().title("Input"));
    frame.render_widget(input, area);
}
