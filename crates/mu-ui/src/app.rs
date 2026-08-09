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
            KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
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
            while let Ok(event) = self.agent_rx.try_recv() {
                self.handle_agent_event(event);
            }

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
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent, width: usize) {
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            (KeyCode::Esc, _) => {
                if self.turn_active {
                    let _ = self.ui_tx.send(UiEvent::Interrupt);
                }
            }
            (KeyCode::Enter, m) if m.contains(KeyModifiers::SHIFT) => self.input.insert_newline(),
            (KeyCode::Enter, _) => self.submit(),
            _ => self.input.handle_key(key, width),
        }
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
            AgentEvent::Error(_) => self.turn_active = false,
            // History-item events are accepted but ignored until rollout
            // step 2 lands the history pane.
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
