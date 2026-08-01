use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::ui::token_usage::render_status_bar;

use super::app::AppState;
use super::input::render_input;
use super::layout::create_layout;
use super::message_list::render_messages;

pub fn run_ui(state: Arc<Mutex<AppState>>) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        // Draw
        {
            let state = state.lock().unwrap();
            terminal.draw(|frame| {
                let (msg_area, input_area, status_area) = create_layout(frame.area());

                render_messages(frame, msg_area, &state);
                render_input(frame, input_area, &state);
                render_status_bar(frame, status_area, &state);
            })?;

            if state.should_exit {
                break;
            }
        }

        // Handle input
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                let mut state = state.lock().unwrap();
                handle_key(&mut state, key);
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    Ok(())
}

fn handle_key(state: &mut AppState, key: event::KeyEvent) {
    // Handle approval prompts first
    if state.pending_approval.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                if let Some(ref approval) = state.pending_approval {
                    *approval.response.lock().unwrap() = Some(true);
                }
                state.pending_approval = None;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                if let Some(ref approval) = state.pending_approval {
                    *approval.response.lock().unwrap() = Some(false);
                }
                state.pending_approval = None;
            }
            _ => {}
        }
        return;
    }

    // Normal input handling
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_exit = true;
        }
        KeyCode::Char(c) if !state.loading => {
            state.input.insert(state.cursor, c);
            state.cursor += 1;
        }
        KeyCode::Backspace if !state.loading && state.cursor > 0 => {
            state.cursor -= 1;
            state.input.remove(state.cursor);
        }
        KeyCode::Left if state.cursor > 0 => {
            state.cursor -= 1;
        }
        KeyCode::Right if state.cursor < state.input.len() => {
            state.cursor += 1;
        }
        KeyCode::Enter if !state.loading && !state.input.is_empty() => {
            // Submit the input - handled by the main loop
            let text = state.input.clone();
            state.messages.push(super::app::DisplayMessage {
                role: "user".into(),
                content: text.clone(),
            });
            state.pending_submit = Some(text);
            state.input.clear();
            state.cursor = 0;
            state.loading = true;
        }
        KeyCode::Up => {
            state.scroll_offset = state.scroll_offset.saturating_add(1);
        }
        KeyCode::Down => {
            state.scroll_offset = state.scroll_offset.saturating_sub(1);
        }
        _ => {}
    }
}
