use mu_ui::{AgentEvent, App, AppConfig, UiEvent, Usage};
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn log(line: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/mu-ui-events.log")
    {
        let _ = writeln!(file, "{line}");
    }
}

fn main() -> std::io::Result<()> {
    let (app, handle, ui_events) = App::new(AppConfig {
        model_display: "(opencode-go) deepseek-v4-flash".to_string(),
        ..AppConfig::default()
    });

    // Per-turn guard so exactly one TurnEnd is emitted: the interrupt path
    // wins the CAS when the user cancels, the timer thread wins when the
    // turn runs to completion.
    let mut turn_ended = Arc::new(AtomicBool::new(false));

    std::thread::spawn(move || {
        while let Ok(event) = ui_events.recv() {
            match event {
                UiEvent::UserPrompt(text) => {
                    log(&format!("UserPrompt: {text:?}"));
                    log("TurnStart");
                    let _ = handle.send(AgentEvent::TurnStart);
                    let h = handle.clone();
                    let ended = Arc::new(AtomicBool::new(false));
                    turn_ended = ended.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        if !ended.swap(true, Ordering::Relaxed) {
                            log("TurnEnd");
                            let _ = h.send(AgentEvent::TurnEnd {
                                usage: Usage {
                                    input_tokens: 12_400,
                                    output_tokens: 3_100,
                                },
                            });
                        }
                    });
                }
                UiEvent::Interrupt => {
                    log("Interrupt");
                    // Esc cancels the in-flight turn, so the turn ends now
                    // (like the real harness emitting TurnEnd on cancel).
                    if !turn_ended.swap(true, Ordering::Relaxed) {
                        log("TurnEnd (interrupted)");
                        let _ = handle.send(AgentEvent::TurnEnd {
                            usage: Usage::default(),
                        });
                    }
                }
            }
        }
        // UI has exited; this must fail with SendError.
        log(&format!(
            "send after exit: {:?}",
            handle.send(AgentEvent::TurnStart)
        ));
    });

    app.run()
}
