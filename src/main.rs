mod theme;
mod ui;
mod wrap;

use std::{env, process, time::Duration};

use async_openai::{
    Client,
    config::{Config, OpenAIConfig},
    types::chat::{
        ChatCompletionMessageToolCalls, ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
        ChatCompletionRequestToolMessage, ChatCompletionRequestUserMessage, ChatCompletionTool,
        ChatCompletionTools, CreateChatCompletionRequestArgs, FinishReason, FunctionObjectArgs,
    },
};
use clap::Parser;
use color_eyre::{
    Result,
    eyre::{bail, eyre},
};
use crossterm::event::{self, Event, KeyEventKind};
use serde_json::json;
use tokio::{fs, process::Command, sync::mpsc, task::JoinSet};
use tokio_util::sync::CancellationToken;

const DEFAULT_INSTRUCTIONS: &str = include_str!("DEFAULT_INSTRUCTIONS.md");

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    // #[arg(short = 'p', long)]
    // prompt: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    console_subscriber::init();
    color_eyre::install()?;
    let _args = Args::parse();
    run_app().await
}

enum AppEvent {
    Key(crossterm::event::KeyEvent),
    Resize,
    AssistantResponse(String),
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
}

enum AIEvent {
    UserPrompt(String),
}

async fn run_app() -> Result<()> {
    let mut join_set = JoinSet::new();
    let token = CancellationToken::new();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AppEvent>();
    let (ai_tx, ai_rx) = mpsc::unbounded_channel::<AIEvent>();

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

    let openai_token = token.clone();
    let openai_events = event_tx.clone();
    join_set.spawn(async move {
        let client = Client::with_config(config);
        openai_stuff(&openai_token, &client, openai_events, ai_rx).await
    });

    // Spawn a blocking task for crossterm events — event::poll and event::read
    // are blocking calls and must NOT run on an async tokio task.
    let event_tx = event_tx.clone();
    let event_token = token.clone();
    join_set.spawn_blocking(move || {
        while !event_token.is_cancelled() {
            if let Ok(true) = event::poll(std::time::Duration::from_millis(100)) {
                match event::read() {
                    Ok(Event::Key(key)) => {
                        if key.kind == KeyEventKind::Press
                            && event_tx.send(AppEvent::Key(key)).is_err()
                        {
                            break;
                        }
                    }
                    Ok(Event::Resize(_, _)) => {
                        if event_tx.send(AppEvent::Resize).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
        Ok(())
    });

    let mut terminal = ratatui::init();
    let result = ui::App::new(event_rx, ai_tx).run(&mut terminal).await;
    token.cancel();
    if tokio::time::timeout(Duration::from_secs(5), join_set.join_all())
        .await
        .is_err()
    {
        eprintln!("graceful shutdown timed out");
    };
    ratatui::restore();
    result
}

async fn openai_stuff<T: Config>(
    token: &CancellationToken,
    client: &Client<T>,
    tx_events: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    mut rx_events: tokio::sync::mpsc::UnboundedReceiver<AIEvent>,
) -> Result<()> {
    // indoc can probably b
    let mut messages: Vec<ChatCompletionRequestMessage> =
        vec![ChatCompletionRequestSystemMessage::from(DEFAULT_INSTRUCTIONS).into()];

    while !token.is_cancelled() {
        let event = match rx_events.recv().await {
            Some(event) => event,
            None => break,
        };

        match event {
            AIEvent::UserPrompt(prompt) => {
                messages.push(ChatCompletionRequestUserMessage::from(prompt).into())
            }
        }

        let turn_result: Result<()> = async {
            for _i in 0..20 {
                if token.is_cancelled() {
                    return Ok(());
                }

                let request = CreateChatCompletionRequestArgs::default()
                    .max_completion_tokens(512u32)
                    .model("deepseek-v4-flash-free")
                    .messages(messages.clone())
                    .tools(vec![
                        ChatCompletionTools::Function(ChatCompletionTool {
                            function: FunctionObjectArgs::default()
                                .name("read")
                                .description("Read and return the contents of a file")
                                .parameters(json!({
                                    "type": "object",
                                    "properties": {
                                        "file_path": {
                                            "type": "string",
                                            "description": "The path to the file to read"
                                        }
                                    },
                                    "required": ["file_path"]
                                }))
                                .strict(true)
                                .build()?,
                        }),
                        ChatCompletionTools::Function(ChatCompletionTool {
                            function: FunctionObjectArgs::default()
                                .name("write")
                                .description("Write content to a file")
                                .parameters(json!({
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
                                }))
                                .strict(true)
                                .build()?,
                        }),
                        ChatCompletionTools::Function(ChatCompletionTool {
                            function: FunctionObjectArgs::default()
                                .name("bash")
                                .description("Execute a shell command")
                                .parameters(json!({
                                    "type": "object",
                                    "required": ["command"],
                                    "properties": {
                                        "command": {
                                        "type": "string",
                                        "description": "The command to execute"
                                        }
                                    }
                                }))
                                .strict(true)
                                .build()?,
                        }),
                        ChatCompletionTools::Function(ChatCompletionTool {
                            function: FunctionObjectArgs::default()
                                .name("fetch")
                                .description("Fetch the HTML content of a website")
                                .parameters(json!({
                                    "type": "object",
                                    "required": ["url"],
                                    "properties": {
                                        "url": {
                                            "type": "string",
                                            "description": "The URL of the website to fetch"
                                        }
                                    }
                                }))
                                .strict(true)
                                .build()?,
                        }),
                    ])
                    .build()?;

                let response = client.chat().create(request).await?;

                let Some(choice) = response.choices.first() else {
                    break;
                };

                let message = choice.message.clone();

                if message.content.is_some() || message.tool_calls.is_some() {
                    let mut msg = ChatCompletionRequestAssistantMessageArgs::default();
                    if let Some(tool_calls) = message.tool_calls.clone() {
                        msg.tool_calls(tool_calls);
                    }
                    if let Some(content) = message.content.clone() {
                        msg.content(content);
                    }
                    messages.push(msg.build()?.into());
                }

                if let Some(content) = &message.content
                    && !content.is_empty()
                {
                    let _ = tx_events.send(AppEvent::AssistantResponse(content.clone()));
                }

                match choice.finish_reason {
                    Some(FinishReason::Stop) | Some(FinishReason::Length) => {
                        break;
                    }
                    _ => {}
                }

                let Some(tool_calls) = message.tool_calls else {
                    break;
                };

                let mut handles = Vec::new();
                for tool_call_enum in tool_calls {
                    // Extract the function tool call from the enum
                    if let ChatCompletionMessageToolCalls::Function(tool_call) = tool_call_enum {
                        let id = tool_call.id.clone();
                        let name = tool_call.function.name.clone();
                        let args = tool_call.function.arguments.clone();

                        let _ = tx_events.send(AppEvent::ToolCallStart {
                            name: name.clone(),
                            args: args.clone(),
                        });

                        let tx_events = tx_events.clone();
                        let handle = tokio::spawn(async move {
                            let result: Result<String> = call_fn(&name, &args).await;
                            let output = match &result {
                                Ok(output) => output.clone(),
                                Err(err) => err.to_string(),
                            };
                            let success = result.is_ok();
                            let _ = tx_events.send(AppEvent::ToolCallOutput {
                                name: name.clone(),
                                output: output.clone(),
                                success,
                            });
                            let tool_message: ChatCompletionRequestMessage =
                                ChatCompletionRequestToolMessage {
                                    content: output.into(),
                                    tool_call_id: id,
                                }
                                .into();
                            Ok::<ChatCompletionRequestMessage, color_eyre::Report>(tool_message)
                        });
                        handles.push(handle);
                    }
                }

                for handle in handles {
                    if let Ok(Ok(tool_message)) = handle.await {
                        messages.push(tool_message);
                    }
                }
            }
            Ok(())
        }
        .await;
        if let Err(err) = turn_result {
            let debug = format!(
                "{}\nMessages: {}",
                err,
                serde_json::to_string_pretty(&messages).unwrap_or_default()
            );
            let _ = tx_events.send(AppEvent::Error(debug));
        }
    }

    Ok(())
}

async fn call_fn(name: &str, args: &str) -> Result<String> {
    let arguments: serde_json::Value = serde_json::from_str(args)
        .map_err(|e| eyre!("failed to parse tool arguments as JSON: {e}"))?;

    match name {
        "read" => {
            let file_path = arguments["file_path"]
                .as_str()
                .ok_or_else(|| eyre!("tool 'read' requires a string field 'file_path'"))?;
            let content = fs::read_to_string(file_path).await?;
            Ok(content)
        }
        "write" => {
            let file_path = arguments["file_path"]
                .as_str()
                .ok_or_else(|| eyre!("tool 'write' requires a string field 'file_path'"))?;
            let content = arguments["content"]
                .as_str()
                .ok_or_else(|| eyre!("tool 'write' requires a string field 'content'"))?;
            fs::write(file_path, content).await?;
            Ok(content.to_string())
        }
        "bash" => {
            let command = arguments["command"]
                .as_str()
                .ok_or_else(|| eyre!("tool 'bash' requires a string field 'command'"))?;
            let output = Command::new("bash").arg("-c").arg(command).output().await?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("command exited with {}: {stderr}", output.status);
            }
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        "fetch" => {
            let url = arguments["url"]
                .as_str()
                .ok_or_else(|| eyre!("tool 'fetch' requires a string field 'url'"))?;
            let response = reqwest::get(url).await?;
            response.text().await.map_err(Into::into)
        }
        _ => Err(eyre!("attempted to call unknown tool '{name}'")),
    }
}
