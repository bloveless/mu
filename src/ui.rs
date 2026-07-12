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


use color_eyre::{Result, eyre::eyre};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::{DefaultTerminal, Frame};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tokio_util::sync::CancellationToken;

use crate::theme::Theme;
use crate::wrap::wrap;
use crate::{AIEvent, AppEvent};

enum Focus {
    History,
    Input,
}

enum HistoryItem {
    UserPrompt(String),
    AssistantResponse(String),
    SystemError(String),
    ToolCallStart {
        name: String,
        args: String,
    },
    ToolCallOutput {
        name: String,
        output: String,
        success: bool,
    },
}

pub struct App {
    events: mpsc::UnboundedReceiver<crate::AppEvent>,
    history: Vec<HistoryItem>,
    scrollbar_state: ScrollbarState,
    content_length: usize,
    viewport_length: usize,
    follow_bottom: bool,
    input: String,
    cursor: usize,
    focus: Focus,
    /// True while an agent turn is in flight, drives the "working…" indicator.
    working: bool,
    /// The turn-scoped cancellation token for the turn currently in flight;
    /// `None` when idle. The agent holds a clone of the same token.
    current_turn: Option<CancellationToken>,
    ai_events: mpsc::UnboundedSender<crate::AIEvent>,
}

impl App {
    pub fn new(
        events: mpsc::UnboundedReceiver<crate::AppEvent>,
        ai_events: mpsc::UnboundedSender<crate::AIEvent>,
    ) -> Self {
        Self {
            events,
            history: Vec::new(),
            scrollbar_state: ScrollbarState::new(0),
            content_length: 0,
            viewport_length: 0,
            follow_bottom: true,
            input: String::new(),
            cursor: 0,
            focus: Focus::Input,
            working: false,
            current_turn: None,
            ai_events,
        }
    }

    pub async fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            terminal.draw(|frame| self.render(frame))?;

            // Block on the first event (so the UI sleeps when idle), then drain
            // everything currently queued in one pass. Collapsing a burst of
            // agent output (streamed tokens / many tool results) into a single
            // redraw keeps typing and scrolling responsive as history grows.
            let maybe_event = timeout(Duration::from_millis(250), self.events.recv()).await;
            match maybe_event {
                Ok(Some(first)) => {
                    let mut quit = false;
                    let mut batch = vec![first];
                    while let Ok(ev) = self.events.try_recv() {
                        batch.push(ev);
                    }
                    for ev in batch {
                        if self.handle_event(ev)? {
                            quit = true;
                            break;
                        }
                    }
                    if quit {
                        return Ok(());
                    }
                }
                Ok(None) => return Ok(()),
                Err(_) => {
                    // No event within the idle timeout; loop around and redraw.
                }
            }
        }
    }

    /// Process one event.
    ///
    /// Returns `Ok(true)` when the UI should quit cleanly, `Ok(false)` to keep
    /// running, and `Err` when the agent has died and the app should exit with
    /// an eyre error the user can read and report.
    fn handle_event(&mut self, event: AppEvent) -> Result<bool> {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Resize => Ok(false),
            AppEvent::AssistantResponse(response) => {
                if !response.is_empty() {
                    self.history.push(HistoryItem::AssistantResponse(response));
                }
                Ok(false)
            }
            AppEvent::Error(error) => {
                if !error.is_empty() {
                    self.history.push(HistoryItem::SystemError(error));
                }
                Ok(false)
            }
            AppEvent::ToolCallStart { name, args } => {
                self.history.push(HistoryItem::ToolCallStart { name, args });
                Ok(false)
            }
            AppEvent::ToolCallOutput {
                name,
                output,
                success,
            } => {
                self.history.push(HistoryItem::ToolCallOutput {
                    name,
                    output,
                    success,
                });
                Ok(false)
            }
            AppEvent::TurnEnd => {
                // Turn finished (or was cancelled): re-enable submission and
                // clear the working indicator.
                self.working = false;
                self.current_turn = None;
                Ok(false)
            }
            AppEvent::Fatal(msg) => Err(eyre!(msg)),
        }
    }

    /// Process one key event. See `handle_event` for the return contract.
    /// Typing and focus changes remain available whether or not a turn is
    /// in flight.
    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        match self.focus {
            Focus::Input => match (key.code, key.modifiers) {
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => Ok(true),
                (KeyCode::Esc, _) => {
                    // Cancel the turn currently in flight, if any.
                    if let Some(token) = self.current_turn.take() {
                        token.cancel();
                    }
                    // Clear optimistically for instant feedback; the agent's
                    // TurnEnd confirms it idempotently.
                    self.working = false;
                    Ok(false)
                }
                (KeyCode::Char('j' | 'k'), KeyModifiers::CONTROL) => {
                    self.toggle_focus();
                    Ok(false)
                }
                (KeyCode::Char(char), _) => {
                    // Always allow editing the input box, even mid-turn.
                    self.input.insert(self.cursor, char);
                    self.cursor += char.len_utf8();
                    Ok(false)
                }
                (KeyCode::Enter, _) if !self.input.is_empty() && !self.working => {
                    let turn_token = CancellationToken::new();
                    self.current_turn = Some(turn_token.clone());
                    self.working = true;
                    let prompt = self.input.clone();
                    if self
                        .ai_events
                        .send(AIEvent::UserPrompt(prompt.clone(), turn_token))
                        .is_err()
                    {
                        // The agent task has gone away; surface a readable error
                        // rather than silently swallowing it.
                        self.working = false;
                        self.current_turn = None;
                        return Err(eyre!(
                            "agent task is no longer running; cannot send user prompt"
                        ));
                    }
                    self.history.push(HistoryItem::UserPrompt(prompt));
                    self.input.clear();
                    self.cursor = 0;
                    Ok(false)
                }
                (KeyCode::Backspace, _) if self.cursor > 0 => {
                    self.cursor = self.input.floor_char_boundary(self.cursor - 1);
                    self.input.remove(self.cursor);
                    Ok(false)
                }
                (KeyCode::Delete, _) if self.cursor < self.input.len() => {
                    self.input.remove(self.cursor);
                    Ok(false)
                }
                (KeyCode::Left, _) if self.cursor > 0 => {
                    self.cursor = self.input.floor_char_boundary(self.cursor - 1);
                    Ok(false)
                }
                (KeyCode::Right, _) if self.cursor < self.input.len() => {
                    self.cursor = self.input.ceil_char_boundary(self.cursor + 1);
                    Ok(false)
                }
                (KeyCode::Home, _) => {
                    self.cursor = 0;
                    Ok(false)
                }
                (KeyCode::End, _) => {
                    self.cursor = self.input.len();
                    Ok(false)
                }
                _ => Ok(false),
            },
            Focus::History => match (key.code, key.modifiers) {
                (KeyCode::Char('j' | 'k'), KeyModifiers::CONTROL) => {
                    self.toggle_focus();
                    Ok(false)
                }
                (KeyCode::Char('q') | KeyCode::Esc, _) => Ok(true),
                (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => {
                    self.scroll_down();
                    Ok(false)
                }
                (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => {
                    self.scroll_up();
                    Ok(false)
                }
                (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                    self.scroll_page_up();
                    Ok(false)
                }
                (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                    self.scroll_page_down();
                    Ok(false)
                }
                _ => Ok(false),
            },
        }
    }

    fn toggle_focus(&mut self) {
        match self.focus {
            Focus::Input => {
                self.follow_bottom = false;
                self.focus = Focus::History;
            }
            Focus::History => {
                self.follow_bottom = true;
                self.focus = Focus::Input;
            }
        }
    }

    /// Scroll up by one line.
    fn scroll_up(&mut self) {
        self.scrollbar_state.prev();
        self.follow_bottom = false;
    }

    /// Scroll down by one line.
    fn scroll_down(&mut self) {
        // Clamp so the last viewport worth of content is the final position.
        let max = self.content_length.saturating_sub(self.viewport_length);
        let position = (self.scrollbar_state.get_position() + 1).min(max);
        self.scrollbar_state = self.scrollbar_state.position(position);
        self.follow_bottom = false;
    }

    /// Scroll up by half a viewport.
    fn scroll_page_up(&mut self) {
        let delta = self.viewport_length / 2;
        let position = self.scrollbar_state.get_position().saturating_sub(delta);
        self.scrollbar_state = self.scrollbar_state.position(position);
        self.follow_bottom = false;
    }

    /// Scroll down by half a viewport.
    fn scroll_page_down(&mut self) {
        let delta = self.viewport_length / 2;
        let max = self.content_length.saturating_sub(self.viewport_length);
        let position = (self.scrollbar_state.get_position() + delta).min(max);
        self.scrollbar_state = self.scrollbar_state.position(position);
        self.follow_bottom = false;
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

    /// Render the conversation history with scroll support.
    fn render_content(&mut self, frame: &mut Frame, area: Rect) {
        let mut content = Vec::new();
        for item in &self.history {
            match item {
                HistoryItem::UserPrompt(text) => {
                    content.push(Line::from(Span::styled("▌ USER ", Theme::user_tag())));
                    for l in wrap(text, area.width.saturating_sub(2)) {
                        content.push(Line::from(Span::styled(
                            format!("  {l}"),
                            Theme::user_text(),
                        )));
                    }
                    content.push(Line::default());
                }
                HistoryItem::AssistantResponse(text) => {
                    content.push(Line::from(Span::styled("▌ ASSISTANT ", Theme::agent_tag())));
                    for l in wrap(text, area.width.saturating_sub(2)) {
                        content.push(Line::from(Span::styled(
                            format!("  {l}"),
                            Theme::agent_text(),
                        )));
                    }
                    content.push(Line::default());
                }
                HistoryItem::SystemError(text) => {
                    content.push(Line::from(Span::styled("▌ ERROR ", Theme::system_error())));
                    for l in wrap(text, area.width.saturating_sub(2)) {
                        content.push(Line::from(Span::styled(
                            format!("  {l}"),
                            Theme::system_error(),
                        )));
                    }
                    content.push(Line::default());
                }
                HistoryItem::ToolCallStart { name, args } => {
                    content.push(Line::from(Span::styled(
                        format!("▌ {name} "),
                        Theme::tool_badge_running(),
                    )));
                    for l in wrap(args, area.width.saturating_sub(2)) {
                        content.push(Line::from(Span::styled(
                            format!("  {l}"),
                            Theme::tool_args(),
                        )));
                    }
                    content.push(Line::default());
                }
                HistoryItem::ToolCallOutput {
                    name,
                    output,
                    success,
                } => {
                    let badge_style = if *success {
                        Theme::tool_badge_success()
                    } else {
                        Theme::tool_badge_failed()
                    };
                    content.push(Line::from(Span::styled(format!("▌ {name} "), badge_style)));
                    for l in wrap(output, area.width.saturating_sub(2)) {
                        content.push(Line::from(Span::styled(
                            format!("  {l}"),
                            Theme::tool_output(),
                        )));
                    }
                    content.push(Line::default());
                }
            }
        }
        let content_length = content.len();
        let viewport_length = area.height as usize;
        let max_position = content_length.saturating_sub(viewport_length);
        let position = if self.follow_bottom {
            max_position
        } else {
            self.scrollbar_state.get_position().min(max_position)
        };
        self.content_length = content_length;
        self.viewport_length = viewport_length;
        // Update the existing state rather than rebuilding from scratch.
        self.scrollbar_state = self
            .scrollbar_state
            .content_length(content_length)
            .viewport_content_length(viewport_length)
            .position(position);
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
        let before = &self.input[..self.cursor];
        let after = &self.input[self.cursor..];
        let block = if self.working {
            Block::default()
                .borders(Borders::TOP)
                .border_style(Theme::agent_tag())
                .title(Line::from(Span::styled(
                    " ● working… ",
                    Theme::agent_tag(),
                )))
        } else {
            Block::default().border_style(style).borders(Borders::TOP)
        };
        let input = Paragraph::new(Line::from(vec![
            Span::raw(before),
            Span::styled("▌", Theme::border_active()),
            Span::raw(after),
        ]))
        .block(block);
        frame.render_widget(input, area);
    }
}
