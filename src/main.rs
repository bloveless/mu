mod theme;
mod ui;
mod wrap;

use std::{env, process};

use async_openai::{
    Client,
    config::{Config, OpenAIConfig},
};
use clap::Parser;
use color_eyre::Result;
use crossterm::event::{self, Event};
use serde_json::{Value, json};
use tokio::{fs, process::Command, sync::mpsc};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    // #[arg(short = 'p', long)]
    // prompt: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let _args = Args::parse();
    let result = run_app().await;
    println!("after run_app");
    result
}

enum AppEvent {
    Tick,
    Key(crossterm::event::KeyEvent),
}

async fn run_app() -> Result<()> {
    // Channel to bridge async inputs/events to the UI loop
    let (tx, rx) = mpsc::unbounded_channel::<AppEvent>();
    let token = CancellationToken::new();

    let _args = Args::parse();

    let base_url =
        env::var("OPENCODE_BASE_URL").unwrap_or_else(|_| "https://opencode.ai/zen/v1".to_string());

    let api_key = env::var("OPENCODE_API_KEY").unwrap_or_else(|_| {
        eprintln!("OPENCODE_API_KEY is not set");
        process::exit(1);
    });

    let config = OpenAIConfig::new()
        .with_api_base(base_url)
        .with_api_key(api_key);

    let ai_event_tx = tx.clone();
    let openai_token = token.clone();
    tokio::task::spawn(async move {
        let client = Client::with_config(config);
        openai_stuff(&openai_token, &client, &ai_event_tx).await
    });

    // Spawn a blocking task for crossterm events — event::poll and event::read
    // are blocking calls and must NOT run on an async tokio task.
    let event_tx = tx.clone();
    let event_token = token.clone();
    tokio::task::spawn_blocking(move || {
        while !event_token.is_cancelled() {
            if let Ok(true) = event::poll(std::time::Duration::from_millis(50)) {
                match event::read() {
                    Ok(Event::Key(key)) => {
                        if event_tx.send(AppEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            // Send periodic tick to update timers/animations
            if event_tx.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });

    let mut terminal = ratatui::init();
    let result = ui::App::new(rx).run(&mut terminal).await;
    ratatui::restore();
    token.cancel();
    result
}

async fn openai_stuff<T: Config>(
    token: &CancellationToken,
    client: &Client<T>,
    _ai_event_tx: &mpsc::UnboundedSender<AppEvent>,
) -> Result<()> {
    let mut messages = vec![json!({
        "role": "user",
        "content": "Sup",
    })];

    for i in 0..20 {
        if token.is_cancelled() {
            eprintln!("agent loop cancelled");
            return Ok(());
        }
        eprintln!("Iteration: {}", i);
        let response: Value = client
            .chat()
            .create_byot(json!({
                "messages": messages,
                "tools": [
                    {
                        "type": "function",
                        "function": {
                            "name": "Read",
                            "description": "Read and return the contents of a file",
                            "parameters": {
                                "type": "object",
                                "properties": {
                                    "file_path": {
                                        "type": "string",
                                        "description": "The path to the file to read"
                                    }
                                },
                                "required": ["file_path"]
                            }
                        }
                    },
                    {
                      "type": "function",
                      "function": {
                        "name": "Write",
                        "description": "Write content to a file",
                        "parameters": {
                          "type": "object",
                          "required": ["file_path", "content"],
                          "properties": {
                            "file_path": {
                              "type": "string",
                              "description": "The path of the file to write to"
                            },
                            "content": {
                              "type": "string",
                              "description": "The content to write to the file"
                            }
                          }
                        }
                      }
                    },
                    {
                      "type": "function",
                      "function": {
                        "name": "Bash",
                        "description": "Execute a shell command",
                        "parameters": {
                          "type": "object",
                          "required": ["command"],
                          "properties": {
                            "command": {
                              "type": "string",
                              "description": "The command to execute"
                            }
                          }
                        }
                      }
                    }
                ],
                "model": "deepseek-v4-flash-free",
            }))
            .await?;

        let message = &response["choices"][0]["message"];
        messages.push(serde_json::to_value(message).unwrap());
        let tool_calls = message["tool_calls"].as_array();

        if tool_calls.is_none()
            && let Some(content) = message["content"].as_str()
        {
            println!("{}", content);
            break;
        }

        if let Some(tool_calls) = message["tool_calls"].as_array() {
            for tool_call in tool_calls {
                let id = tool_call["id"].as_str().unwrap();
                let name = tool_call["function"]["name"].as_str().unwrap();
                let arguments = serde_json::from_str::<Value>(
                    tool_call["function"]["arguments"].as_str().unwrap(),
                )?;

                if name == "Read" {
                    let file_path = arguments["file_path"].as_str().unwrap();
                    let content = fs::read_to_string(file_path).await?;
                    eprintln!("Reading file: {}", file_path);
                    messages.push(
                        serde_json::to_value(json!({
                            "role": "tool",
                            "tool_call_id": id,
                            "content": content,
                        }))
                        .unwrap(),
                    );
                }

                if name == "Write" {
                    let file_path = arguments["file_path"].as_str().unwrap();
                    let content = arguments["content"].as_str().unwrap();
                    eprintln!("Writing to file: {}", file_path);
                    fs::write(file_path, content).await?;
                    messages.push(
                        serde_json::to_value(json!({
                            "role": "tool",
                            "tool_call_id": id,
                            "content": content,
                        }))
                        .unwrap(),
                    );
                }

                if name == "Bash" {
                    let command = arguments["command"].as_str().unwrap();
                    eprintln!("Executing command: {}", command);
                    let output = Command::new("bash").arg("-c").arg(command).output().await?;
                    if !output.status.success() {
                        eprintln!(
                            "Command failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                        messages.push(
                            serde_json::to_value(json!({
                                "role": "tool",
                                "tool_call_id": id,
                                "content": String::from_utf8_lossy(&output.stderr),
                            }))
                            .unwrap(),
                        );
                        continue;
                    }

                    messages.push(
                        serde_json::to_value(json!({
                            "role": "tool",
                            "tool_call_id": id,
                            "content": String::from_utf8_lossy(&output.stdout),
                        }))
                        .unwrap(),
                    );
                }
            }
        }
    }

    eprintln!("agent loop exited");
    Ok(())
}
