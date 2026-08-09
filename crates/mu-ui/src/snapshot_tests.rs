//! Golden snapshot tests.
//!
//! These drive the app by pushing [`AgentEvent`]s through the real
//! [`UiHandle`] channel (and, for prompts, the real key path), then render to
//! a fixed-size `TestBackend` and compare the screen against an insta
//! snapshot. The golden files under `src/snapshots/` are the cell-by-cell
//! record of what the UI looks like: review and accept intentional changes
//! with `cargo insta review`, or regenerate with `INSTA_UPDATE=always cargo
//! test`.
//!
//! Colours are intentionally not asserted here (ratatui's `TestBackend`
//! display is text-only); style-level assertions live in `render.rs`'s unit
//! tests.

use insta::assert_snapshot;
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

use crate::{AgentEvent, App, AppConfig, UiHandle, Usage};

/// Deterministic footer context so snapshots don't depend on the machine.
fn config() -> AppConfig {
    AppConfig {
        model_display: "(opencode-go) deepseek-v4-flash".to_string(),
        cwd_display: Some("~/Projects/mu".to_string()),
        branch: Some("main".to_string()),
    }
}

/// The app plus the harness side of the channel.
struct Session {
    app: App,
    handle: UiHandle,
}

impl Session {
    fn new() -> Self {
        let (app, handle, _ui_events) = App::new(config());
        Self { app, handle }
    }

    /// Send an agent event through the real channel and apply it.
    fn push(&mut self, event: AgentEvent) {
        self.handle.send(event).unwrap();
        self.app.drain_events();
    }

    /// Simulate the user typing a prompt and pressing Enter.
    fn prompt(&mut self, text: &str) {
        self.app.input.insert_str(text);
        self.app
            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 78);
    }

    /// Deliver one assistant message whole through the streaming protocol.
    fn message(&mut self, text: &str) {
        self.push(AgentEvent::TurnStart);
        self.push(AgentEvent::MessageStart);
        self.push(AgentEvent::MessageDelta(text.to_string()));
        self.push(AgentEvent::MessageEnd);
        self.push(AgentEvent::TurnEnd {
            usage: Usage {
                input_tokens: 900,
                output_tokens: 500,
            },
        });
    }
}

/// Render the app to a fixed-size test screen and return its text view.
fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::render::draw(frame, app))
        .unwrap();
    terminal.backend().to_string()
}

/// A prompt/reply conversation exercising the message separators, a code
/// fence, wrapping, and a multiline reply.
fn seed_conversation(s: &mut Session) {
    s.prompt("set up a new crate");
    s.message("Created the `mu-ui` crate and wired the channel API. Here is the manifest:");
    s.message("```toml\n[package]\nname = \"mu-ui\"\nedition = \"2024\"\n```");
    s.prompt("how does scrolling work?");
    s.message("Scrolling uses a flattened-line offset model. Each history item reports its wrapped height via `Paragraph::line_count`, cached per item and invalidated only on width change; only the items intersecting the visible window are drawn.");
    s.message("The footer shows the cwd, branch, model, and cumulative token usage, refreshed on turn start and prompt submit rather than per frame.");
}

#[test]
fn empty_ui() {
    let mut session = Session::new();
    assert_snapshot!(render(&mut session.app, 80, 24));
}

#[test]
fn seeded_conversation() {
    let mut session = Session::new();
    seed_conversation(&mut session);
    assert_snapshot!(render(&mut session.app, 80, 24));
}

#[test]
fn seeded_conversation_narrow() {
    let mut session = Session::new();
    seed_conversation(&mut session);
    assert_snapshot!(render(&mut session.app, 40, 24));
}

#[test]
fn scrolled_to_top() {
    let mut session = Session::new();
    seed_conversation(&mut session);
    session.app.sticky_bottom = false;
    session.app.scroll_offset = 0;
    assert_snapshot!(render(&mut session.app, 80, 24));
}

#[test]
fn long_single_message_wraps() {
    let mut session = Session::new();
    let long = vec!["word"; 60].join(" ");
    session.message(&long);
    assert_snapshot!(render(&mut session.app, 60, 24));
}

#[test]
fn streamed_message_deltas() {
    let mut session = Session::new();
    session.push(AgentEvent::TurnStart);
    session.push(AgentEvent::MessageStart);
    session.push(AgentEvent::MessageDelta("One ".to_string()));
    session.push(AgentEvent::MessageDelta("two ".to_string()));
    session.push(AgentEvent::MessageDelta("three.".to_string()));
    session.push(AgentEvent::MessageEnd);
    session.push(AgentEvent::TurnEnd {
        usage: Usage::default(),
    });
    assert_snapshot!(render(&mut session.app, 80, 24));
}
