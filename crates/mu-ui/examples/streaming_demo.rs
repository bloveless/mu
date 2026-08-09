//! Scripted demo of the `mu-ui` harness UI.
//!
//! Rollout step 2: the driver immediately replays a seeded conversation on
//! launch (delivered whole, no streaming yet) so scrollback exists on first
//! paint, and answers every user prompt with a canned reply. Pacing,
//! thinking, tool calls, and `Esc` interruption of the script land in later
//! steps.

use mu_ui::{AgentEvent, App, AppConfig, UiEvent, Usage};

struct Demo {
    handle: mu_ui::UiHandle,
}

impl Demo {
    fn send(&self, event: AgentEvent) {
        let _ = self.handle.send(event);
    }

    /// One assistant turn, delivered whole through the streaming protocol.
    fn message(&self, text: &str) {
        self.send(AgentEvent::TurnStart);
        self.send(AgentEvent::MessageStart);
        self.send(AgentEvent::MessageDelta(text.to_string()));
        self.send(AgentEvent::MessageEnd);
        self.send(AgentEvent::TurnEnd {
            usage: Usage {
                input_tokens: 700,
                output_tokens: 400,
            },
        });
    }

    fn seed(&self) {
        self.message(
            "I created the `mu-ui` crate and wired the channel API: \
             `UiHandle` for agent activity, `Receiver<UiEvent>` for what the \
             user does. Here is the manifest:",
        );
        self.message(
            "```toml\n\
             [package]\n\
             name = \"mu-ui\"\n\
             version = \"0.1.0\"\n\
             edition = \"2024\"\n\
             rust-version = \"1.88\"\n\
             \n\
             [dependencies]\n\
             ratatui = { version = \"0.30.2\", features = [\"unstable-rendered-line-info\"] }\n\
             unicode-width = \"0.2.2\"\n\
             ```",
        );
        self.message(
            "The event loop blocks on `crossterm::event::poll` and drains the \
             agent channel with `try_recv` before each paint, so bursts of \
             deltas batch into a single frame. Terminal setup and teardown \
             happen on the thread that calls `App::run`, and a panic hook \
             restores raw mode, the alternate screen, mouse capture, and \
             bracketed paste on every exit path.",
        );
        self.message(
            "Layout is three vertical bands: the scrollback history, the \
             input band (a 1-4 row multiline editor bounded by horizontal \
             rules), and a two-line footer. Everything is edge-to-edge with \
             no side borders, so terminal selections copy clean text.",
        );
        self.message(
            "Scrolling uses a flattened-line offset model. Each history item \
             reports its wrapped height via `Paragraph::line_count`, cached \
             per item and invalidated only on width change; only the items \
             intersecting the visible window are drawn, and partially visible \
             ones are positioned with `Paragraph::scroll`. This keeps \
             streaming deltas O(visible) — no large offscreen buffer.",
        );
        self.message(
            "The footer recomputes its cwd and git branch on `TurnStart` and \
             on prompt submit rather than per frame, so it stays cheap while \
             the session streams. Cumulative token usage from `TurnEnd` shows \
             on the second line, with the model/provider right-aligned.",
        );
    }

    fn answer(&self, prompt: &str) {
        let reply = format!(
            "Canned reply to \"{prompt}\".\n\n\
             I'm a scripted stand-in for the harness, so I don't really know \
             anything yet — real streaming arrives in a later rollout step. \
             Try scrolling up to browse the seeded conversation, resize the \
             window to watch re-wrap, or submit another prompt."
        );
        self.message(&reply);
    }
}

fn main() -> std::io::Result<()> {
    let (app, handle, ui_events) = App::new(AppConfig {
        model_display: "(opencode-go) deepseek-v4-flash".to_string(),
        ..AppConfig::default()
    });

    // Stand-in for the harness: owns the handle, consumes UI events.
    std::thread::spawn(move || {
        let demo = Demo { handle };
        demo.seed();
        while let Ok(event) = ui_events.recv() {
            match event {
                UiEvent::UserPrompt(text) => demo.answer(&text),
                // Step 5: abandon the in-flight script.
                UiEvent::Interrupt => {}
            }
        }
    });

    app.run()
}
