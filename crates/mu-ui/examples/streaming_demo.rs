//! Scripted demo of the `mu-ui` harness UI.
//!
//! Rollout step 1 wires the channel API and runs the app; the auto-playing
//! transcript and canned streamed replies arrive in later steps.

use mu_ui::{App, AppConfig, UiEvent};

fn main() -> std::io::Result<()> {
    let (app, handle, ui_events) = App::new(AppConfig {
        model_display: "(opencode-go) deepseek-v4-flash".to_string(),
        ..AppConfig::default()
    });

    // Stand-in for the harness: owns the handle, consumes UI events.
    std::thread::spawn(move || {
        let _handle = handle; // drives the scripted transcript in later steps
        while let Ok(event) = ui_events.recv() {
            match event {
                // Step 2: answer prompts with a canned streamed reply.
                UiEvent::UserPrompt(_) => {}
                // Step 5: abandon the in-flight script.
                UiEvent::Interrupt => {}
            }
        }
    });

    app.run()
}
