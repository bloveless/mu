//! `mu-ui` — a synchronous, channel-driven TUI for agent harnesses.
//!
//! The library owns the terminal and the blocking crossterm event loop; the
//! harness drives it entirely through two `std::mpsc` channel ends handed
//! out by [`App::new`]:
//!
//! - [`UiHandle`] — a cloneable `Sender<AgentEvent>` the harness pushes agent
//!   activity (streaming thinking/messages, tool calls, usage, errors) into.
//! - `Receiver<UiEvent>` — what the user did: submitted prompts and
//!   interrupts.
//!
//! No async runtime is held or required; the consumer decides which threads
//! both sides live on.

#![forbid(unsafe_code)]

mod app;
mod history;
mod input;
mod render;
mod theme;

use std::sync::mpsc::{SendError, Sender};

pub use app::{App, AppConfig};

/// Identifier tying a tool call's lifecycle events together: `*Delta`,
/// `Running`, and `End` match their `Start` by id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolCallId(pub String);

impl From<String> for ToolCallId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for ToolCallId {
    fn from(id: &str) -> Self {
        Self(id.to_string())
    }
}

impl std::fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Per-turn token usage reported by the harness.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Everything the harness can tell the UI. The UI is a pure state machine
/// over these events: deltas append to the currently open history item,
/// `*End` seals it.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// A new turn began (also refreshes the footer's cwd/branch).
    TurnStart,
    ThinkingStart,
    ThinkingDelta(String),
    ThinkingEnd,
    MessageStart,
    MessageDelta(String),
    MessageEnd,
    ToolCallStart {
        id: ToolCallId,
        name: String,
    },
    ToolCallArgsDelta {
        id: ToolCallId,
        delta: String,
    },
    ToolCallRunning {
        id: ToolCallId,
    },
    ToolCallEnd {
        id: ToolCallId,
        result: Result<String, String>,
    },
    TurnEnd {
        usage: Usage,
    },
    Error(String),
}

/// Everything the user can tell the harness.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// A submitted prompt (`Enter` on non-empty input).
    UserPrompt(String),
    /// `Esc` while a turn is in flight.
    Interrupt,
}

/// Cloneable handle the harness uses to feed [`AgentEvent`]s into the UI.
/// `Send`, so it can live on any thread.
#[derive(Clone)]
pub struct UiHandle {
    tx: Sender<AgentEvent>,
}

impl UiHandle {
    pub(crate) fn new(tx: Sender<AgentEvent>) -> Self {
        Self { tx }
    }

    /// Push an agent event into the UI. Fails once the UI has exited.
    pub fn send(&self, event: AgentEvent) -> Result<(), SendError<AgentEvent>> {
        self.tx.send(event)
    }
}
