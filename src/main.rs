use async_openai::{Client, config::OpenAIConfig};
use clap::Parser;
use serde_json::{Value, json};
use std::{env, process};
use tokio::fs;

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    #[arg(short = 'p', long)]
    prompt: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let base_url = env::var("OPENROUTER_BASE_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string());

    let api_key = env::var("OPENROUTER_API_KEY").unwrap_or_else(|_| {
        eprintln!("OPENROUTER_API_KEY is not set");
        process::exit(1);
    });

    let config = OpenAIConfig::new()
        .with_api_base(base_url)
        .with_api_key(api_key);

    let client = Client::with_config(config);

    let mut messages = vec![json!({
        "role": "user",
        "content": args.prompt,
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
                    }
                ],
                // "model": "deepseek-v4-flash",
                "model": "anthropic/claude-haiku-4.5",
            }))
            .await?;

        // You can use print statements as follows for debugging, they'll be visible when running tests.
        eprintln!("Logs from your program will appear here!");

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
            }
        }
    }

    Ok(())
}
