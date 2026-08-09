use tokio_util::sync::CancellationToken;

pub enum AppEvent {
    /// Terminal key press (TUI only; never produced in JSON mode).
    Key(crossterm::event::KeyEvent),
    /// Text pasted by the terminal (bracketed paste).
    Paste(String),
    /// Terminal resize (TUI only).
    Resize,
    ThinkingChunkReceived(String),
    ChunkReceived(String),
    Error(String),
    ToolCallStart {
        name: String,
        args: String,
    },
    ToolCallOutput {
        name: String,
        output: String,
        success: bool,
    },
    UsageReceived {
        prompt_tokens: u32,
        completion_tokens: u32,
        total_tokens: u32,
    },
    /// The agent finished a turn: it stopped, hit the iteration cap, was
    /// cancelled by the user, or hit a per-turn error already surfaced as
    /// `AppEvent::Error`. The UI uses this to clear the "working…" indicator
    /// and re-enable prompt submission.
    TurnEnd,
    /// The agent task is terminating with an unrecoverable error. The UI should
    /// exit and surface `msg` to the user via eyre so it can be reported.
    Fatal(String),
}

pub enum AIEvent {
    /// A new user prompt paired with a turn-scoped cancellation token. The UI
    /// keeps a clone of the same token so it can cancel just this in-flight
    /// turn with the Esc key without affecting other turns or app lifetime.
    UserPrompt(String, CancellationToken),
}
