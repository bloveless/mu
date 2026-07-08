mod theme;
mod ui;
mod wrap;

use async_openai::{Client, config::OpenAIConfig};
use clap::Parser;
use color_eyre::Result;
use crossterm::event::{self, KeyCode};
use ratatui::widgets::ScrollbarState;
use serde_json::{Value, json};
use std::{env, process};
use tokio::{fs, process::Command};

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    // #[arg(short = 'p', long)]
    // prompt: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let base_url =
        env::var("OPENCODE_BASE_URL").unwrap_or_else(|_| "https://opencode.ai/zen/v1".to_string());

    let api_key = env::var("OPENCODE_API_KEY").unwrap_or_else(|_| {
        eprintln!("OPENCODE_API_KEY is not set");
        process::exit(1);
    });

    let config = OpenAIConfig::new()
        .with_api_base(base_url)
        .with_api_key(api_key);

    let client = Client::with_config(config);

    color_eyre::install()?;

    let mut vertical = ScrollbarState::new(0);
    return ratatui::run(|terminal| {
        loop {
            terminal.draw(|frame| ui::render(frame, &mut vertical))?;
            if let Some(key) = event::read()?.as_key_press_event() {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                    KeyCode::Char('j') | KeyCode::Down => vertical.next(),
                    KeyCode::Char('k') | KeyCode::Up => vertical.prev(),
                    _ => {}
                }
            }
        }
    });

    let mut messages = vec![json!({
        "role": "user",
        "content": "Sup",
    })];

    for i in 0..20 {
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

        if tool_calls.is_none() {
            if let Some(content) = message["content"].as_str() {
                println!("{}", content);
                break;
            }
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

    Ok(())
}
