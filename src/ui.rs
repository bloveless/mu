//! UI for the AI coding assistant.
//!
//! Uses a `ScrollView` widget (from `tui-scrollview`) for the conversation
//! history area.  The `ScrollView` provides its own vertical scrollbar, which
//! replaces the manual `Scrollbar` + `ScrollbarState` the previous version of
//! this code maintained.
//!
//! # Performance notes
//!
//! Word-wrapping every message's text on every redraw is the single biggest
//! performance sink as the conversation grows.  We cache the wrapped lines
//! inside each `HistoryItem` so that only *new* or *re-sized* messages pay
//! the wrapping cost.  The total rendered height and the input-area height
//! are also cached to avoid re-measuring on every frame.

use color_eyre::{Result, eyre::eyre};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect, Size};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::sync::mpsc::Receiver;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

use crate::theme::Theme;
use crate::wrap::wrap;
use crate::{AIEvent, AppEvent};

/// Padding reserved inside the conversation history area:
///
///   2 columns for the "  " line prefix (indent)
///   1 column for the vertical scrollbar (when visible)
///   2 columns of breathing room so content never abuts the scrollbar
const CONTENT_PADDING: u16 = 5;

/// Minimum height (in terminal rows) for the prompt input area.
const INPUT_MIN_HEIGHT: u16 = 3;

/// Maximum height (in terminal rows) for the prompt input area.
const INPUT_MAX_HEIGHT: u16 = 10;

enum Focus {
    History,
    Input,
}

// ---------------------------------------------------------------------------
// WrappedItem – caches the word-wrapped lines for a single piece of text
// ---------------------------------------------------------------------------

/// A block of text whose word-wrapped lines are cached and invalidated when
/// the wrapping width changes (e.g. on terminal resize).
///
/// Wrapping walks every character in the text to find word boundaries and
/// measure widths, so it's by far the most expensive operation during a
/// redraw.  Caching the result turns O(n) per redraw into O(1) for every
/// item that hasn't changed.
struct WrappedItem {
    text: String,
    /// Lines after word-wrapping to `wrapped_at_width`.  Empty until the
    /// first call to `ensure_wrapped_at`.
    lines: Vec<String>,
    /// The width at which `lines` was produced, or `None` if the cache is
    /// empty / has been invalidated.
    wrapped_at_width: Option<u16>,
}

impl WrappedItem {
    fn new(text: String) -> Self {
        Self {
            text,
            lines: Vec::new(),
            wrapped_at_width: None,
        }
    }

    /// Return the wrapped lines, computing them from the raw text if the
    /// cache is stale or empty.
    fn ensure_wrapped_at(&mut self, width: u16) -> &[String] {
        if self.wrapped_at_width != Some(width) {
            self.lines = wrap(&self.text, width);
            self.wrapped_at_width = Some(width);
        }
        &self.lines
    }

    /// Invalidate the cache so the next `ensure_wrapped_at` re-wraps.
    fn invalidate(&mut self) {
        self.wrapped_at_width = None;
        self.lines.clear();
    }
}

// ---------------------------------------------------------------------------
// HistoryItem
// ---------------------------------------------------------------------------

enum HistoryItem {
    UserPrompt(WrappedItem),
    AssistantResponse(WrappedItem),
    SystemError(WrappedItem),
    ToolCallStart {
        name: String,
        args: WrappedItem,
    },
    ToolCallOutput {
        name: String,
        output: WrappedItem,
        success: bool,
    },
}

impl HistoryItem {
    /// Return the already-wrapped lines for this item.  **Must** have called
    /// `ensure_all_wrapped` first.
    fn wrapped_lines(&self) -> &[String] {
        match self {
            HistoryItem::UserPrompt(w)
            | HistoryItem::AssistantResponse(w)
            | HistoryItem::SystemError(w) => &w.lines,
            HistoryItem::ToolCallStart { args, .. } => &args.lines,
            HistoryItem::ToolCallOutput { output, .. } => &output.lines,
        }
    }

    /// The rendered height in rows: one tag line + wrapped content + one
    /// trailing blank line.  **Must** have called `ensure_all_wrapped` first.
    fn rendered_height(&self) -> u16 {
        1 + self.wrapped_lines().len() as u16 + 1
    }

    /// Invalidate the wrap caches inside this item.
    fn invalidate_wrap(&mut self) {
        match self {
            HistoryItem::UserPrompt(w)
            | HistoryItem::AssistantResponse(w)
            | HistoryItem::SystemError(w) => w.invalidate(),
            HistoryItem::ToolCallStart { args, .. } => args.invalidate(),
            HistoryItem::ToolCallOutput { output, .. } => output.invalidate(),
        }
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    events: Receiver<crate::AppEvent>,
    history: Vec<HistoryItem>,
    scroll_view_state: ScrollViewState,
    /// Cached scroll view buffer, rebuilt only when content or layout
    /// changes.
    scroll_view: Option<ScrollView>,
    /// True when the scroll view's content is stale and must be rebuilt.
    scroll_view_dirty: bool,
    /// Cached total rendered height of all history items at the current
    /// wrap width.  Avoids re-measuring every item on every frame.
    cached_total_height: u16,
    /// The wrap width used for `cached_total_height`.  `None` = invalid.
    cached_total_height_width: Option<u16>,
    /// True while the view should automatically stick to the bottom of the
    /// conversation history. The user is at the bottom while typing (Input
    /// focus) and can scroll away by pressing j/k/Up/Down etc. in History
    /// focus. Re-entering Input focus re-enables follow-bottom.
    follow_bottom: bool,
    input: String,
    /// Cached height of the input area.  Invalidated (set to 0) whenever
    /// the input text or the terminal width changes.
    cached_input_height: u16,
    /// The terminal width used to compute `cached_input_height`.
    cached_input_height_width: u16,
    cursor: usize,
    focus: Focus,
    /// True while an agent turn is in flight, drives the "working…"
    /// indicator.
    working: bool,
    /// The turn-scoped cancellation token for the turn currently in flight;
    /// `None` when idle. The agent holds a clone of the same token.
    current_turn: Option<CancellationToken>,
    /// Sender for pushing user prompts to the async agent. This is a *tokio*
    /// unbounded sender only because the agent needs to `select!` over its
    /// receiver; `send` is itself synchronous, so the (sync) UI thread can
    /// hold it without touching the runtime.
    ai_events: UnboundedSender<crate::AIEvent>,
}

impl App {
    pub fn new(
        events: Receiver<crate::AppEvent>,
        ai_events: UnboundedSender<crate::AIEvent>,
    ) -> Self {
        Self {
            events,
            history: Vec::new(),
            scroll_view_state: ScrollViewState::default(),
            scroll_view: None,
            scroll_view_dirty: true,
            cached_total_height: 0,
            cached_total_height_width: None,
            follow_bottom: true,
            input: String::new(),
            cached_input_height: INPUT_MIN_HEIGHT,
            cached_input_height_width: 0,
            cursor: 0,
            focus: Focus::Input,
            working: false,
            current_turn: None,
            ai_events,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            terminal.draw(|frame| self.render(frame))?;

            // Block on the first event (so the UI sleeps when idle), then
            // drain everything currently queued in one pass. Collapsing a
            // burst of agent output (streamed tokens / many tool results)
            // into a single redraw keeps typing and scrolling responsive as
            // history grows.
            //
            // `recv_timeout` is a blocking std call — this thread is fully
            // synchronous and never touches the tokio runtime. The idle
            // timeout doubles as the redraw tick (e.g. for a future
            // blinking cursor).
            match self.events.recv_timeout(Duration::from_millis(250)) {
                Ok(first) => {
                    let mut quit = false;
                    let mut batch = Vec::with_capacity(4);
                    batch.push(first);
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
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // No event within the idle window; loop around and
                    // redraw (e.g. for a blinking cursor, even though we
                    // don't have one yet).
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    // Both forwarders (agent + crossterm) have dropped their
                    // senders. Treat that as a clean shutdown.
                    return Ok(());
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Event handling
    // ------------------------------------------------------------------

    /// Process one event.
    ///
    /// Returns `Ok(true)` when the UI should quit cleanly, `Ok(false)` to
    /// keep running, and `Err` when the agent has died and the app should
    /// exit with an eyre error the user can read and report.
    fn handle_event(&mut self, event: AppEvent) -> Result<bool> {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Resize => {
                // Invalidate all wrap caches; the terminal width changed so
                // every message needs to be re-wrapped.
                for item in &mut self.history {
                    item.invalidate_wrap();
                }
                self.cached_total_height_width = None;
                self.scroll_view_dirty = true;
                // Input height cache also depends on terminal width.
                self.cached_input_height_width = 0;
                Ok(false)
            }
            AppEvent::AssistantResponse(response) if !response.is_empty() => {
                self.history
                    .push(HistoryItem::AssistantResponse(WrappedItem::new(response)));
                self.cached_total_height_width = None;
                self.scroll_view_dirty = true;
                self.scroll_to_bottom_if_following();
                Ok(false)
            }
            AppEvent::Error(error) if !error.is_empty() => {
                self.history
                    .push(HistoryItem::SystemError(WrappedItem::new(error)));
                self.cached_total_height_width = None;
                self.scroll_view_dirty = true;
                self.scroll_to_bottom_if_following();
                Ok(false)
            }
            AppEvent::ToolCallStart { name, args } => {
                self.history.push(HistoryItem::ToolCallStart {
                    name,
                    args: WrappedItem::new(args),
                });
                self.cached_total_height_width = None;
                self.scroll_view_dirty = true;
                self.scroll_to_bottom_if_following();
                Ok(false)
            }
            AppEvent::ToolCallOutput {
                name,
                output,
                success,
            } => {
                self.history.push(HistoryItem::ToolCallOutput {
                    name,
                    output: WrappedItem::new(output),
                    success,
                });
                self.cached_total_height_width = None;
                self.scroll_view_dirty = true;
                self.scroll_to_bottom_if_following();
                Ok(false)
            }
            AppEvent::TurnEnd => {
                // Turn finished (or was cancelled): re-enable submission
                // and clear the working indicator.
                self.working = false;
                self.current_turn = None;
                Ok(false)
            }
            AppEvent::Fatal(msg) => Err(eyre!(msg)),
            // Non-empty guard above catches the non-empty case; empty
            // responses / errors are no-ops.
            AppEvent::AssistantResponse(_) | AppEvent::Error(_) => Ok(false),
        }
    }

    /// If the user is currently following the bottom of the conversation,
    /// scroll the view to the bottom so the next render shows the latest
    /// content.
    fn scroll_to_bottom_if_following(&mut self) {
        if self.follow_bottom {
            self.scroll_view_state.scroll_to_bottom();
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
                    // Clear optimistically for instant feedback; the
                    // agent's TurnEnd confirms it idempotently.
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
                    self.cached_input_height_width = 0; // invalidate cache
                    Ok(false)
                }
                // Enter submits the prompt (only when idle with input).
                (KeyCode::Enter, KeyModifiers::NONE) if !self.input.is_empty() && !self.working => {
                    self.submit_prompt()
                }
                // Supporting both alt+enter and shift+enter since alt+enter is
                // more compatible with different terminals that suppor the Kitty
                // keyboard protocol.
                (KeyCode::Enter, m) if m == KeyModifiers::ALT || m == KeyModifiers::SHIFT => {
                    self.input.insert(self.cursor, '\n');
                    self.cursor += '\n'.len_utf8();
                    self.cached_input_height_width = 0;
                    Ok(false)
                }
                (KeyCode::Backspace, _) if self.cursor > 0 => {
                    self.cursor = self.input.floor_char_boundary(self.cursor - 1);
                    self.input.remove(self.cursor);
                    self.cached_input_height_width = 0;
                    Ok(false)
                }
                (KeyCode::Delete, _) if self.cursor < self.input.len() => {
                    self.input.remove(self.cursor);
                    self.cached_input_height_width = 0;
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
                (KeyCode::Char('q'), _) => Ok(true),
                (KeyCode::Esc, _) => {
                    // Esc in history mode returns focus to the input area
                    // rather than quitting.
                    self.focus = Focus::Input;
                    self.follow_bottom = true;
                    self.scroll_to_bottom_if_following();
                    Ok(false)
                }
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

    /// Submit the current input buffer as a user prompt.
    ///
    /// Called when the user presses Enter in Input focus.  Returns an
    /// error if the agent task has gone away, which causes the app to
    /// exit.
    fn submit_prompt(&mut self) -> Result<bool> {
        let turn_token = CancellationToken::new();
        self.current_turn = Some(turn_token.clone());
        self.working = true;
        let prompt = std::mem::take(&mut self.input);
        self.cursor = 0;
        self.cached_input_height_width = 0; // input cleared

        if self
            .ai_events
            .send(AIEvent::UserPrompt(prompt.clone(), turn_token))
            .is_err()
        {
            self.working = false;
            self.current_turn = None;
            return Err(eyre!(
                "agent task is no longer running; cannot send user prompt"
            ));
        }

        self.history
            .push(HistoryItem::UserPrompt(WrappedItem::new(prompt)));
        self.cached_total_height_width = None;
        self.scroll_view_dirty = true;
        self.scroll_to_bottom_if_following();
        Ok(false)
    }

    fn toggle_focus(&mut self) {
        match self.focus {
            Focus::Input => {
                self.focus = Focus::History;
            }
            Focus::History => {
                self.focus = Focus::Input;
                self.follow_bottom = true;
                self.scroll_to_bottom_if_following();
            }
        }
    }

    /// Scroll up by one line.
    fn scroll_up(&mut self) {
        self.scroll_view_state.scroll_up();
        self.follow_bottom = false;
    }

    /// Scroll down by one line.
    fn scroll_down(&mut self) {
        self.scroll_view_state.scroll_down();
        self.follow_bottom = false;
    }

    /// Scroll up by half a viewport.
    fn scroll_page_up(&mut self) {
        self.scroll_view_state.scroll_page_up();
        self.follow_bottom = false;
    }

    /// Scroll down by half a viewport.
    fn scroll_page_down(&mut self) {
        self.scroll_view_state.scroll_page_down();
        self.follow_bottom = false;
    }

    // ------------------------------------------------------------------
    // Rendering
    // ------------------------------------------------------------------

    /// Render the UI.
    fn render(&mut self, frame: &mut Frame) {
        let input_height = self.compute_input_height(frame.area().width);
        let [top_bar, main, prompt] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(input_height),
        ])
        .areas(frame.area());

        let style = match self.focus {
            Focus::Input => Style::default(),
            Focus::History => Style::default().yellow(),
        };

        frame.render_widget(
            Block::default()
                .title(" μ ")
                .title_alignment(Alignment::Center)
                .border_style(style)
                .borders(Borders::TOP),
            top_bar,
        );

        self.render_content(frame, main);
        self.render_input(frame, prompt);
    }

    /// Determine how many terminal rows the prompt input area should
    /// occupy.
    ///
    /// Grows with the wrapped content height but is clamped to keep the
    /// history area usable.  Results are cached and only recomputed when
    /// the input text or terminal width changes.
    fn compute_input_height(&mut self, area_width: u16) -> u16 {
        if self.input.is_empty() {
            return INPUT_MIN_HEIGHT;
        }
        // Cache hit: same terminal width since the last input edit.
        if self.cached_input_height_width == area_width {
            return self.cached_input_height;
        }
        let wrapped = wrap(&self.input, area_width.max(1));
        let h = (wrapped.len() as u16 + 1).clamp(INPUT_MIN_HEIGHT, INPUT_MAX_HEIGHT);
        self.cached_input_height = h;
        self.cached_input_height_width = area_width;
        h
    }

    /// Render the conversation history into a `ScrollView` with its
    /// built-in scrollbar.  Rebuilds the scroll view buffer from scratch
    /// only when the content or the container width has changed.
    fn render_content(&mut self, frame: &mut Frame, area: Rect) {
        if self.history.is_empty() {
            // Clear the scroll view so stale state doesn't linger when
            // history is empty (e.g. before any prompts).
            self.scroll_view = None;
            return;
        }

        let wrap_width = area.width.saturating_sub(CONTENT_PADDING).max(1);

        // ----- ensure every item has wrapped lines at this width -----
        // This is cheap: only items whose cached width differs from
        // `wrap_width` (new items, or after a resize) actually re-wrap.
        // Everything else is a no-op length check.
        let total_height = self.ensure_all_wrapped(wrap_width);
        if total_height == 0 {
            return;
        }

        // ----- (re)build the scroll view if stale -----
        let size = Size::new(area.width, total_height);
        if self.scroll_view_dirty || self.scroll_view.as_ref().is_none_or(|sv| sv.size() != size) {
            self.rebuild_scroll_view(area.width, wrap_width, size);
        }

        // ----- render the visible portion -----
        frame.render_stateful_widget(
            self.scroll_view.as_ref().unwrap(),
            area,
            &mut self.scroll_view_state,
        );
    }

    /// Ensure every history item has its wrapped lines cached at
    /// `wrap_width`.  Returns the total rendered height of all items.
    ///
    /// When the cached total height is still valid (same width), this
    /// returns immediately without touching any item.
    fn ensure_all_wrapped(&mut self, wrap_width: u16) -> u16 {
        if self.cached_total_height_width == Some(wrap_width) {
            return self.cached_total_height;
        }

        let mut h = 0u16;
        for item in &mut self.history {
            // This triggers wrapping only for items whose cached width
            // differs from `wrap_width`.
            match item {
                HistoryItem::UserPrompt(w)
                | HistoryItem::AssistantResponse(w)
                | HistoryItem::SystemError(w) => {
                    w.ensure_wrapped_at(wrap_width);
                }
                HistoryItem::ToolCallStart { args, .. } => {
                    args.ensure_wrapped_at(wrap_width);
                }
                HistoryItem::ToolCallOutput { output, .. } => {
                    output.ensure_wrapped_at(wrap_width);
                }
            }
            h = h.saturating_add(item.rendered_height());
        }

        self.cached_total_height = h;
        self.cached_total_height_width = Some(wrap_width);
        h
    }

    /// Rebuild the scroll view buffer from scratch.  This is the expensive
    /// path, but we only reach it when content *actually* changes (new
    /// items, resize), not on every frame.
    fn rebuild_scroll_view(&mut self, area_width: u16, _wrap_width: u16, size: Size) {
        let mut scroll_view = ScrollView::new(size)
            .vertical_scrollbar_visibility(ScrollbarVisibility::Automatic)
            .horizontal_scrollbar_visibility(ScrollbarVisibility::Never);

        let mut y = 0u16;
        for item in &self.history {
            match item {
                HistoryItem::UserPrompt(w) => {
                    y = render_tagged_block(
                        &mut scroll_view,
                        y,
                        area_width,
                        "▌ USER ",
                        Theme::user_tag(),
                        w,
                        Theme::user_text(),
                    );
                }
                HistoryItem::AssistantResponse(w) => {
                    y = render_tagged_block(
                        &mut scroll_view,
                        y,
                        area_width,
                        "▌ ASSISTANT ",
                        Theme::agent_tag(),
                        w,
                        Theme::agent_text(),
                    );
                }
                HistoryItem::SystemError(w) => {
                    y = render_tagged_block(
                        &mut scroll_view,
                        y,
                        area_width,
                        "▌ ERROR ",
                        Theme::system_error(),
                        w,
                        Theme::system_error(),
                    );
                }
                HistoryItem::ToolCallStart { name, args } => {
                    let tag = format!("▌ {name} ");
                    y = render_tagged_block(
                        &mut scroll_view,
                        y,
                        area_width,
                        &tag,
                        Theme::tool_badge_running(),
                        args,
                        Theme::tool_args(),
                    );
                }
                HistoryItem::ToolCallOutput {
                    name,
                    output,
                    success,
                } => {
                    let tag = format!("▌ {name} ");
                    let badge_style = if *success {
                        Theme::tool_badge_success()
                    } else {
                        Theme::tool_badge_failed()
                    };
                    y = render_tagged_block(
                        &mut scroll_view,
                        y,
                        area_width,
                        &tag,
                        badge_style,
                        output,
                        Theme::tool_output(),
                    );
                }
            }
        }

        self.scroll_view = Some(scroll_view);
        self.scroll_view_dirty = false;
    }

    /// Render the prompt input area with multiline support.
    ///
    /// The cursor is shown inline (▌) at the current byte position, and
    /// the input text wraps naturally.  A top border separates the prompt
    /// from the history area above.
    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let style = match self.focus {
            Focus::Input => Style::default().yellow(),
            Focus::History => Style::default(),
        };

        let block = if self.working {
            Block::default()
                .borders(Borders::TOP)
                .border_style(Theme::agent_tag())
                .title(Line::from(Span::styled(" ● working… ", Theme::agent_tag())))
        } else {
            Block::default().border_style(style).borders(Borders::TOP)
        };

        // Determine which logical line (by '\n') the cursor is on and the
        // column offset within that line.
        let (cursor_line, cursor_col) = self.cursor_line_col();

        let line_count = self.input.split('\n').count();
        let mut lines = Vec::with_capacity(line_count);
        for (i, text_line) in self.input.split('\n').enumerate() {
            if i == cursor_line {
                let col = cursor_col.min(text_line.len());
                let before = &text_line[..col];
                let after = &text_line[col..];
                lines.push(Line::from(vec![
                    Span::raw(before),
                    Span::styled("▌", Theme::border_active()),
                    Span::raw(after),
                ]));
            } else {
                lines.push(Line::from(Span::raw(text_line)));
            }
        }

        let input = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block);
        frame.render_widget(input, area);
    }

    /// Return the (line_index, column) of `self.cursor` in the input text,
    /// where lines are separated by `'\n'`.
    fn cursor_line_col(&self) -> (usize, usize) {
        let preceding = &self.input[..self.cursor];
        let last_newline = preceding.rfind('\n');
        let line = preceding.matches('\n').count();
        let col = match last_newline {
            Some(pos) => self.cursor - pos - 1,
            None => self.cursor,
        };
        (line, col)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Render one tagged block (tag line + wrapped text + trailing blank line)
/// into `scroll_view` starting at row `y`. Returns the next free row.
///
/// The `text` argument must already have its wrapped lines computed at the
/// correct width — this function does *not* call `wrap` on it.
fn render_tagged_block(
    scroll_view: &mut ScrollView,
    y: u16,
    outer_width: u16,
    tag: &str,
    tag_style: Style,
    text: &WrappedItem,
    text_style: Style,
) -> u16 {
    let wrapped = &text.lines;

    // Tag line
    let tag_paragraph = Paragraph::new(Line::from(Span::styled(tag, tag_style)));
    scroll_view.render_widget(tag_paragraph, Rect::new(0, y, outer_width, 1));

    // Wrapped text lines — use the pre-cached wrapped lines directly.
    if !wrapped.is_empty() {
        // Avoid the `format!("  {l}")` allocation on every line: prepend
        // two spaces as a separate `Span::raw` so `l` is used verbatim.
        let lines: Vec<Line> = wrapped
            .iter()
            .map(|l| Line::from(vec![Span::raw("  "), Span::styled(l.as_str(), text_style)]))
            .collect();
        let text_paragraph = Paragraph::new(lines);
        scroll_view.render_widget(
            text_paragraph,
            Rect::new(0, y + 1, outer_width, wrapped.len() as u16),
        );
    }

    // Return the next free row (tag + text + blank line)
    y.saturating_add(1) // tag
        .saturating_add(wrapped.len() as u16) // text
        .saturating_add(1) // blank line
}
