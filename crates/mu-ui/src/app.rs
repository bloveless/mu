//! Event loop, terminal setup/teardown, and input dispatch.

use std::{
    io,
    panic::PanicHookInfo,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{Receiver, Sender},
    },
    time::Duration,
};

use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        event::{
            self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
            EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
            KeyboardEnhancementFlags, MouseEvent, MouseEventKind, PopKeyboardEnhancementFlags,
            PushKeyboardEnhancementFlags,
        },
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};

use crate::{
    AgentEvent, UiEvent, UiHandle, Usage, history::History, input::Input, render, theme::Theme,
};

/// How long the event loop may block on crossterm input before waking to
/// drain pending [`AgentEvent`]s. Caps streaming updates at ~20 fps.
const TICK: Duration = Duration::from_millis(50);

/// Static configuration handed to [`App::new`].
#[derive(Debug, Clone, Default)]
pub struct AppConfig {
    /// Right half of footer line 2, e.g. `"(opencode-go) deepseek-v4-flash"`.
    pub model_display: String,
    /// Footer working-directory override; computed home-relative from the
    /// process cwd when `None`.
    pub cwd_display: Option<String>,
    /// Footer git-branch override; discovered by walking up to `.git/HEAD`
    /// when `None`.
    pub branch: Option<String>,
}

/// The TUI application; owns the terminal while running. See the crate docs.
pub struct App {
    pub(crate) config: AppConfig,
    pub(crate) theme: Theme,
    pub(crate) input: Input,
    pub(crate) history: History,
    pub(crate) usage: Usage,
    pub(crate) cwd_display: String,
    pub(crate) branch: Option<String>,
    agent_rx: Receiver<AgentEvent>,
    ui_tx: Sender<UiEvent>,
    turn_active: bool,
    should_quit: bool,
    /// Top flat line of the visible history window (the flattened-line offset
    /// model drives scroll; see [`crate::render`]).
    pub(crate) scroll_offset: usize,
    /// Auto-follow the bottom; any up-scroll disengages, scrolling to the
    /// bottom (or `G`) re-engages.
    pub(crate) sticky_bottom: bool,
    /// History pane viewport, cached each frame so key handlers can page.
    pub(crate) history_height: u16,
    /// History pane width, cached each frame so key handlers can measure.
    pub(crate) history_width: u16,
}

impl App {
    /// Create the app plus the harness-side channel ends: a [`UiHandle`] to
    /// push [`AgentEvent`]s into and a [`Receiver<UiEvent>`] to read user
    /// actions out of.
    pub fn new(config: AppConfig) -> (Self, UiHandle, Receiver<UiEvent>) {
        let (agent_tx, agent_rx) = std::sync::mpsc::channel();
        let (ui_tx, ui_rx) = std::sync::mpsc::channel();
        let mut app = Self {
            config,
            theme: Theme::default(),
            input: Input::default(),
            history: History::default(),
            usage: Usage::default(),
            cwd_display: String::new(),
            branch: None,
            agent_rx,
            ui_tx,
            turn_active: false,
            should_quit: false,
            scroll_offset: 0,
            sticky_bottom: true,
            history_height: 0,
            history_width: 0,
        };
        app.refresh_context();
        (app, UiHandle::new(agent_tx), ui_rx)
    }

    /// Take over the calling thread: set up the terminal (raw mode,
    /// alternate screen, mouse capture, bracketed paste), run the event
    /// loop, and restore the terminal on every exit path, including panic.
    ///
    /// Returns when the user quits (`/quit` or `Ctrl+C`).
    pub fn run(mut self) -> io::Result<()> {
        let _guard = TerminalGuard::install()?;
        let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        self.event_loop(&mut terminal)
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<()> {
        while !self.should_quit {
            // Drain pending agent events before each paint so bursts of
            // streaming deltas batch into a single frame.
            self.drain_events();

            terminal.draw(|frame| render::draw(frame, self))?;

            // Display width of the input text area (two columns are the
            // prompt glyph); Up/Down row-walking needs it.
            let width = terminal
                .size()
                .map_or(1, |size| size.width.saturating_sub(2).max(1) as usize);

            if event::poll(TICK)? {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        self.handle_key(key, width)
                    }
                    Event::Paste(text) => self.input.insert_str(&text),
                    Event::Mouse(MouseEvent {
                        kind: MouseEventKind::ScrollUp,
                        ..
                    }) => self.scroll_by(-3),
                    Event::Mouse(MouseEvent {
                        kind: MouseEventKind::ScrollDown,
                        ..
                    }) => self.scroll_by(3),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent, width: usize) {
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            (KeyCode::Esc, _) => {
                if self.turn_active {
                    self.history.seal_open();
                    let _ = self.ui_tx.send(UiEvent::Interrupt);
                }
            }
            (KeyCode::Enter, m) if m.contains(KeyModifiers::SHIFT) => self.input.insert_newline(),
            (KeyCode::Enter, _) => self.submit(),
            _ => {
                // History keys only when the input is empty; otherwise they
                // type into the buffer. Mouse wheel scrolls regardless.
                if self.input.is_empty() && self.handle_history_key(key) {
                    return;
                }
                self.input.handle_key(key, width);
            }
        }
    }

    /// History-scroll keybindings: `j`/`Down` and `k`/`Up` by line, `PgUp`/
    /// `PgDn` by page, `g` top, `G` bottom. Returns whether the key was
    /// handled as a history key.
    fn handle_history_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
            KeyCode::PageUp => self.scroll_by(-(self.history_height.max(1) as isize)),
            KeyCode::PageDown => self.scroll_by(self.history_height.max(1) as isize),
            KeyCode::Char('g') => self.scroll_to_top(),
            KeyCode::Char('G') => self.scroll_to_bottom(),
            _ => return false,
        }
        true
    }

    /// Move the history window by `delta` flat lines (negative scrolls up,
    /// disengaging the sticky bottom; reaching the bottom re-engages it).
    fn scroll_by(&mut self, delta: isize) {
        let max_offset = self.history_max_offset();
        let next = (self.scroll_offset as isize + delta).clamp(0, max_offset as isize) as usize;
        self.scroll_offset = next;
        self.sticky_bottom = next == max_offset;
    }

    fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
        self.sticky_bottom = false;
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.history_max_offset();
        self.sticky_bottom = true;
    }

    fn history_max_offset(&mut self) -> usize {
        let viewport = self.history_height.max(1) as usize;
        self.history
            .total_height(self.history_width)
            .saturating_sub(viewport)
    }

    fn submit(&mut self) {
        let Some(text) = self.input.take() else {
            return;
        };
        if let Some(command) = text.strip_prefix('/')
            && self.run_command(command)
        {
            return;
        }
        self.refresh_context();
        self.history.user_prompt(text.clone());
        let _ = self.ui_tx.send(UiEvent::UserPrompt(text));
    }

    /// Slash-command registry; returns whether the command was handled.
    /// `/quit` is the only v1 command — the seam for future ones.
    fn run_command(&mut self, command: &str) -> bool {
        match command.trim() {
            "quit" => {
                self.should_quit = true;
                true
            }
            _ => false,
        }
    }

    /// Drain any pending [`AgentEvent`]s from the channel into the UI state.
    /// Called each frame by the event loop; exposed crate-wide so the snapshot
    /// test harness can drive the app through the real [`UiHandle`] channel
    /// without owning the terminal.
    pub(crate) fn drain_events(&mut self) {
        while let Ok(event) = self.agent_rx.try_recv() {
            self.handle_agent_event(event);
        }
    }

    fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TurnStart => {
                self.turn_active = true;
                self.refresh_context();
            }
            AgentEvent::TurnEnd { usage } => {
                self.turn_active = false;
                self.usage.input_tokens += usage.input_tokens;
                self.usage.output_tokens += usage.output_tokens;
            }
            AgentEvent::MessageStart => self.history.message_start(),
            AgentEvent::MessageDelta(delta) => self.history.message_delta(&delta),
            AgentEvent::MessageEnd => self.history.message_end(),
            AgentEvent::Error(_) => self.turn_active = false,
            // Thinking and tool-call events land in rollout steps 3–4.
            _ => {}
        }
    }

    /// Refresh the footer's cwd/branch. Runs at startup, on each turn start,
    /// and on each prompt submit — never per frame.
    fn refresh_context(&mut self) {
        self.cwd_display = match &self.config.cwd_display {
            Some(display) => display.clone(),
            None => home_relative_cwd(),
        };
        self.branch = match &self.config.branch {
            Some(branch) => Some(branch.clone()),
            None => git_branch(),
        };
    }
}

/// Current working directory with the user's home collapsed to `~`.
fn home_relative_cwd() -> String {
    let cwd = std::env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "?".to_string());
    let Some(home) = std::env::var_os("HOME") else {
        return cwd;
    };
    let home = home.to_string_lossy();
    if cwd == *home {
        return "~".to_string();
    }
    if let Some(rest) = cwd.strip_prefix(&*home)
        && rest.starts_with('/')
    {
        return format!("~{rest}");
    }
    cwd
}

/// Current git branch, found by walking up from the cwd to `.git/HEAD`.
/// Handles worktrees (where `.git` is a `gitdir:` file) and detached HEAD
/// (reported as a short hash).
fn git_branch() -> Option<String> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let dotgit = dir.join(".git");
        if dotgit.is_dir() {
            return read_head(&dotgit.join("HEAD"));
        }
        if dotgit.is_file() {
            let pointer = std::fs::read_to_string(&dotgit).ok()?;
            let gitdir = pointer.trim().strip_prefix("gitdir:")?.trim();
            let gitdir = if Path::new(gitdir).is_absolute() {
                PathBuf::from(gitdir)
            } else {
                dir.join(gitdir)
            };
            return read_head(&gitdir.join("HEAD"));
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn read_head(path: &Path) -> Option<String> {
    let head = std::fs::read_to_string(path).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: refs/heads/") {
        Some(branch) => Some(branch.to_string()),
        None => Some(head.chars().take(8).collect()),
    }
}

type PanicHook = Box<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>;

/// Restores the terminal on drop — and on panic, via a hook that restores
/// before chaining to the previously installed hook.
struct TerminalGuard {
    prev_hook: Arc<PanicHook>,
}

impl TerminalGuard {
    fn install() -> io::Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        ) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        // Lets supporting terminals report Shift+Enter distinctly. Ignored
        // elsewhere, where Shift+Enter degrades to Enter.
        let _ = execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );

        let prev_hook = Arc::new(std::panic::take_hook());
        let hook = prev_hook.clone();
        std::panic::set_hook(Box::new(move |info| {
            Self::restore();
            hook(info);
        }));
        Ok(Self { prev_hook })
    }

    fn restore() {
        let _ = execute!(
            io::stdout(),
            PopKeyboardEnhancementFlags,
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        Self::restore();
        // During a panic the installed hook has already restored the
        // terminal and chained to the previous hook; std forbids
        // take_hook/set_hook from a panicking thread (doing so panics
        // again and aborts), so leave the hook installed while unwinding.
        if std::thread::panicking() {
            return;
        }
        let _ = std::panic::take_hook(); // discard ours
        let prev_hook = self.prev_hook.clone();
        std::panic::set_hook(Box::new(move |info| prev_hook(info)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UiHandle, history::HistoryItem};

    fn new_app() -> (App, UiHandle, Receiver<UiEvent>) {
        App::new(AppConfig::default())
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Tall enough that scrolling means something at an 80-col, 10-row
    /// viewport: two wrapped messages around ~13 rows each.
    fn seed_tall_history(app: &mut App) {
        app.history.user_prompt("short prompt".to_string());
        app.history.message_start();
        app.history.message_delta(&"b ".repeat(500));
        app.history.message_end();
        app.history.user_prompt("another prompt".to_string());
        app.history.message_start();
        app.history.message_delta(&"d ".repeat(500));
        app.history.message_end();
    }

    #[test]
    fn submit_pushes_prompt_item_and_emits_event() {
        let (mut app, _handle, ui_events) = new_app();
        app.input.insert_str("hello");
        app.handle_key(key(KeyCode::Enter), 78);
        assert_eq!(app.history.items().len(), 1);
        assert!(matches!(
            &app.history.items()[0],
            HistoryItem::UserPrompt(t) if t.as_str() == "hello"
        ));
        assert!(matches!(
            ui_events.try_recv(),
            Ok(UiEvent::UserPrompt(t)) if t == "hello"
        ));
    }

    #[test]
    fn slash_quit_is_not_added_to_history() {
        let (mut app, _handle, _ui_events) = new_app();
        app.input.insert_str("/quit");
        app.handle_key(key(KeyCode::Enter), 78);
        assert!(app.should_quit);
        assert!(app.history.items().is_empty());
    }

    #[test]
    fn message_events_build_and_merge_history() {
        let (mut app, _handle, _ui_events) = new_app();
        app.handle_agent_event(AgentEvent::MessageStart);
        app.handle_agent_event(AgentEvent::MessageDelta("hello ".into()));
        app.handle_agent_event(AgentEvent::MessageDelta("world".into()));
        app.handle_agent_event(AgentEvent::MessageEnd);
        assert_eq!(app.history.items().len(), 1);
        assert!(matches!(
            &app.history.items()[0],
            HistoryItem::Message(t) if t.as_str() == "hello world"
        ));
    }

    #[test]
    fn delta_without_start_implicitly_starts_a_message() {
        let (mut app, _handle, _ui_events) = new_app();
        app.handle_agent_event(AgentEvent::MessageDelta("stray".into()));
        app.handle_agent_event(AgentEvent::MessageEnd);
        assert!(matches!(
            &app.history.items()[0],
            HistoryItem::Message(t) if t.as_str() == "stray"
        ));
    }

    #[test]
    fn interrupt_seals_the_open_message() {
        let (mut app, _handle, _ui_events) = new_app();
        app.turn_active = true;
        app.handle_agent_event(AgentEvent::MessageStart);
        app.handle_agent_event(AgentEvent::MessageDelta("partial".into()));
        app.handle_key(key(KeyCode::Esc), 78);

        // A fresh start must begin a new item, not append to the sealed one.
        app.handle_agent_event(AgentEvent::MessageStart);
        app.handle_agent_event(AgentEvent::MessageDelta("second".into()));
        app.handle_agent_event(AgentEvent::MessageEnd);
        assert_eq!(app.history.items().len(), 2);
        assert!(matches!(
            &app.history.items()[0],
            HistoryItem::Message(t) if t.as_str() == "partial"
        ));
        assert!(matches!(
            &app.history.items()[1],
            HistoryItem::Message(t) if t.as_str() == "second"
        ));
    }

    /// Empty input: `Up`/`k` scroll the history up, disengaging the sticky
    /// bottom; `G` returns to the bottom and re-engages it.
    #[test]
    fn empty_input_up_scrolls_history_and_g_reengages() {
        let (mut app, _handle, _ui_events) = new_app();
        seed_tall_history(&mut app);
        app.history_width = 80;
        app.history_height = 10;
        app.scroll_to_bottom();
        assert!(app.sticky_bottom);

        app.handle_key(key(KeyCode::Up), 78);
        assert!(!app.sticky_bottom, "up-scroll disengages follow");
        let max = app.history_max_offset();
        assert!(app.scroll_offset < max);
        let scrolled = app.scroll_offset;

        app.handle_key(key(KeyCode::Char('k')), 78);
        assert_eq!(app.scroll_offset, scrolled - 1, "k scrolls up a line");

        app.handle_key(key(KeyCode::Char('j')), 78);
        app.handle_key(key(KeyCode::Char('G')), 78);
        assert!(app.sticky_bottom, "G re-engages follow");
        let max = app.history_max_offset();
        assert_eq!(app.scroll_offset, max);
    }

    #[test]
    fn g_goes_to_top() {
        let (mut app, _handle, _ui_events) = new_app();
        seed_tall_history(&mut app);
        app.history_width = 80;
        app.history_height = 10;
        app.scroll_to_bottom();
        app.handle_key(key(KeyCode::Char('g')), 78);
        assert_eq!(app.scroll_offset, 0);
        assert!(!app.sticky_bottom);
    }

    #[test]
    fn nonempty_input_up_walks_input_not_history() {
        let (mut app, _handle, _ui_events) = new_app();
        seed_tall_history(&mut app);
        app.history_width = 80;
        app.history_height = 10;
        app.scroll_to_bottom();
        let offset_before = app.scroll_offset;

        app.input.insert_str("abc");
        app.handle_key(key(KeyCode::Up), 78);
        assert_eq!(
            app.scroll_offset, offset_before,
            "input Up must not scroll history"
        );
    }

    #[test]
    fn page_keys_scroll_by_viewport() {
        let (mut app, _handle, _ui_events) = new_app();
        seed_tall_history(&mut app);
        app.history_width = 80;
        app.history_height = 10;
        app.scroll_to_bottom();
        let max = app.scroll_offset;

        app.handle_key(key(KeyCode::PageUp), 78);
        assert_eq!(app.scroll_offset, max.saturating_sub(10));
        app.handle_key(key(KeyCode::PageDown), 78);
        assert_eq!(app.scroll_offset, max);
        assert!(app.sticky_bottom, "page-down to the bottom re-engages");
    }

    /// Scroll-up then new content: the scrolled-up viewport must not jump
    /// (the sticky bottom's per-frame pinning lives in `render::draw_history`
    /// and is covered there).
    #[test]
    fn scroll_up_then_new_content_keeps_viewport() {
        let (mut app, _handle, _ui_events) = new_app();
        seed_tall_history(&mut app);
        app.history_width = 80;
        app.history_height = 10;
        app.scroll_to_bottom();

        app.handle_key(key(KeyCode::Up), 78);
        let keep = app.scroll_offset;
        app.history.message_start();
        app.history.message_delta(&"more ".repeat(200));
        app.history.message_end();
        assert_eq!(
            app.scroll_offset, keep,
            "scrolled-up viewport must not jump"
        );
        assert!(keep < app.history_max_offset());
    }
}
