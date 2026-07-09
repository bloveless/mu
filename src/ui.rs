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

use color_eyre::Result;
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::{DefaultTerminal, Frame};
use tokio::sync::mpsc;

use crate::AppEvent;
use crate::theme::Theme;
use crate::wrap::wrap;

enum Focus {
    History,
    Input,
}

pub struct App<'a> {
    events: mpsc::UnboundedReceiver<crate::AppEvent>,
    history: Vec<Line<'a>>,
    scrollbar_state: ScrollbarState,
    input: String,
    focus: Focus,
}

impl<'a> App<'a> {
    pub fn new(events: mpsc::UnboundedReceiver<crate::AppEvent>) -> Self {
        Self {
            events,
            history: Vec::new(),
            scrollbar_state: ScrollbarState::new(0),
            input: String::new(),
            focus: Focus::Input,
        }
    }

    pub async fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let result = loop {
            terminal.draw(|frame| self.render(frame))?;

            if let Some(event) = self.events.recv().await
                && let AppEvent::Key(key) = event
            {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                    KeyCode::Char('j') | KeyCode::Down => self.scrollbar_state.next(),
                    KeyCode::Char('k') | KeyCode::Up => self.scrollbar_state.prev(),
                    _ => {}
                }
            }
        };
        println!("Exiting app loop");
        result
    }

    /// Render the UI with vertical/horizontal scrollbars.
    fn render(&mut self, frame: &mut Frame) {
        let [top_bar, main, prompt] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(5),
        ])
        .areas(frame.area());
        let style = match self.focus {
            Focus::Input => Style::default(),
            Focus::History => Style::default().yellow(),
        };

        let [content, scrollbar] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(1)])
            .spacing(2)
            .areas(main);

        frame.render_widget(
            Block::default().border_style(style).borders(Borders::TOP),
            top_bar,
        );

        self.render_content(frame, content);
        self.render_vertical_scrollbar(frame, scrollbar);
        self.render_input(frame, prompt);
    }

    /// Render a vertical scrollbar on the right side of the area.
    fn render_vertical_scrollbar(&mut self, frame: &mut Frame, area: Rect) {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        frame.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut self.scrollbar_state,
        );
    }

    /// Render some content.
    fn render_content(&self, frame: &mut Frame, area: Rect) {
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
            self.scrollbar_state.get_position().to_string().yellow(),
        ]));
        content.push(Line::default());
        frame.render_widget(
            Paragraph::new(content)
                .wrap(Wrap { trim: false })
                .scroll((self.scrollbar_state.get_position() as u16, 0)),
            area,
        );
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let style = match self.focus {
            Focus::Input => Style::default().yellow(),
            Focus::History => Style::default(),
        };
        let input = Paragraph::new(self.input.clone())
            .block(Block::default().border_style(style).borders(Borders::TOP));
        frame.render_widget(input, area);
    }
}
