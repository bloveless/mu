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

/// Identifies this bot to HTTP services (e.g. crates.io) that require a
/// meaningful `User-Agent` rather than the bare reqwest default.
const HTTP_USER_AGENT: &str = concat!(
    env!("CARGO_PKG_NAME"),
    "/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/bloveless/mu)"
);

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
    /// The agent finished a turn: it stopped, hit the iteration cap, was
    /// cancelled by the user, or hit a per-turn error already surfaced as
    /// `AppEvent::Error`. The UI uses this to clear the "working…" indicator
    /// and re-enable prompt submission.
    TurnEnd,
    /// The agent task is terminating with an unrecoverable error. The UI should
    /// exit and surface `msg` to the user via eyre so it can be reported.
    Fatal(String),
}

enum AIEvent {
    /// A new user prompt paired with a turn-scoped cancellation token. The UI
    /// keeps a clone of the same token so it can cancel just this in-flight
    /// turn with the Esc key without affecting other turns or app lifetime.
    UserPrompt(String, CancellationToken),
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
    let fatal_events = event_tx.clone();
    join_set.spawn(async move {
        let client = Client::with_config(config);
        let result = openai_stuff(&openai_token, &client, openai_events, ai_rx).await;
        if let Err(err) = &result {
            // The agent is dying; tell the UI so it can exit with a useful
            // message instead of hanging on the next prompt send.
            let _ = fatal_events.send(AppEvent::Fatal(format!("{err:?}")));
        }
        result
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

    loop {
        // Wait for the next user prompt, but bail out immediately if the app
        // is shutting down. The UI also drops `ai_tx` on quit, which drives
        // `recv` to `None` as a backup path.
        let (prompt, turn_token) = tokio::select! {
            ev = rx_events.recv() => match ev {
                Some(AIEvent::UserPrompt(p, t)) => (p, t),
                None => return Ok(()),
            },
            _ = token.cancelled() => return Ok(()),
        };

        messages.push(ChatCompletionRequestUserMessage::from(prompt).into());

        // Remember where the turn began so a cancelled turn can be rolled back
        // out of the conversation history. A half-finished tool_call sequence
        // with no matching tool results would otherwise make the next API
        // request fail validation.
        let checkpoint = messages.len();

        let turn_result: Result<()> = async {
            'turn: for _i in 0..20 {
                if token.is_cancelled() || turn_token.is_cancelled() {
                    break 'turn;
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
                                .name("edit")
                                .description(
                                    "Edit a file with an exact string replacement. \
                                     Provide `old_string` (the exact text to find in the \
                                     file) and `new_string` (the replacement). `old_string` \
                                     must match the file exactly, including whitespace and \
                                     indentation, and must be UNIQUE within the file — if it \
                                     occurs more than once, include more surrounding context \
                                     to make it unique. To CREATE a new file, pass an empty \
                                     `old_string` together with the full file contents as \
                                     `new_string`; this is refused if the file already exists. \
                                     Always read the current file contents before editing it \
                                     so `old_string` matches exactly.",
                                )
                                .parameters(json!({
                                    "type": "object",
                                    "required": ["file_path", "old_string", "new_string"],
                                    "properties": {
                                        "file_path": {
                                            "type": "string",
                                            "description": "The path of the file to edit"
                                        },
                                        "old_string": {
                                            "type": "string",
                                            "description": "The exact text to find in the file. Must be unique within the file unless it is empty, in which case the file is created with `new_string` as its contents (and must not already exist)."
                                        },
                                        "new_string": {
                                            "type": "string",
                                            "description": "The text to replace `old_string` with. When `old_string` is empty, this is the full contents of the new file."
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

                // Bind the chat handle first: `client.chat()` returns a borrowed
                // temporary whose lifetime the create-future relies on, so it
                // must outlive the `select!` polling.
                let chat = client.chat();
                let response = tokio::select! {
                    r = chat.create(request) => r?,
                    _ = token.cancelled() => break 'turn,
                    _ = turn_token.cancelled() => break 'turn,
                };

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

                // Tool calls run on a dedicated JoinSet so the whole batch can
                // be aborted atomically when the app or the current turn is
                // cancelled. Aborting drops the futures, which cancels in-flight
                // `fetch` requests and (via `kill_on_drop`) terminates running
                // `bash` children.
                let mut tool_set: JoinSet<ChatCompletionRequestMessage> = JoinSet::new();
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
                        tool_set.spawn(async move {
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
                            ChatCompletionRequestToolMessage {
                                content: output.into(),
                                tool_call_id: id,
                            }
                            .into()
                        });
                    }
                }

                // Drive the tool batch to completion, but abort everything the
                // instant it is cancelled.
                let cancelled = loop {
                    let next = tokio::select! {
                        n = tool_set.join_next() => n,
                        _ = turn_token.cancelled() => { tool_set.abort_all(); break true; },
                        _ = token.cancelled() => { tool_set.abort_all(); break true; },
                    };
                    match next {
                        Some(Ok(tool_message)) => messages.push(tool_message),
                        // A panicked or aborted tool task: nothing to append;
                        // errors were already surfaced as ToolCallOutput.
                        Some(Err(_)) => {}
                        None => break false,
                    }
                };
                if cancelled {
                    break 'turn;
                }
            }
            Ok(())
        }
        .await;
        if turn_token.is_cancelled() {
            // Roll back any half-finished assistant/tool messages from this
            // cancelled turn so the next prompt starts from a clean state.
            messages.truncate(checkpoint);
        }
        if let Err(err) = turn_result {
            let debug = format!(
                "{}\nMessages: {}",
                err,
                serde_json::to_string_pretty(&messages).unwrap_or_default()
            );
            let _ = tx_events.send(AppEvent::Error(debug));
        }
        // Let the UI know the turn is over either way so it can clear the
        // "working…" indicator and re-enable prompt submission.
        let _ = tx_events.send(AppEvent::TurnEnd);
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
        "edit" => {
            let file_path = arguments["file_path"]
                .as_str()
                .ok_or_else(|| eyre!("tool 'edit' requires a string field 'file_path'"))?;
            let old_string = arguments["old_string"]
                .as_str()
                .ok_or_else(|| eyre!("tool 'edit' requires a string field 'old_string'"))?;
            let new_string = arguments["new_string"]
                .as_str()
                .ok_or_else(|| eyre!("tool 'edit' requires a string field 'new_string'"))?;

            if old_string.is_empty() {
                // Create-a-new-file path. An empty `old_string` is the 
                // convention for "this file doesn't exist yet"; refuse to 
                // clobber an existing file so the model can't accidentally 
                // blank out content it meant to edit.
                if fs::try_exists(file_path).await? {
                    bail!(
                        "tool 'edit' cannot create '{file_path}': file already \
                         exists. To edit it, provide a non-empty, unique \
                         `old_string` that matches the current contents \
                         exactly."
                    );
                }
                // Create parent directories so a new file can be added in a 
                // new subdirectory in a single step, mirroring `write`.
                if let Some(parent) = std::path::Path::new(file_path).parent()
                    && !parent.as_os_str().is_empty() {
                        fs::create_dir_all(parent).await?;
                    }
                fs::write(file_path, new_string).await?;
                Ok(format!("Created {file_path}"))
            } else {
                let content = fs::read_to_string(file_path).await?;
                match content.matches(old_string).count() {
                    0 => bail!(
                        "tool 'edit': `old_string` was not found in \
                         '{file_path}'. Make sure it matches the file \
                         exactly, including whitespace and indentation."
                    ),
                    1 => {
                        let updated = content.replacen(old_string, new_string, 1);
                        fs::write(file_path, updated).await?;
                        Ok(format!("Edited {file_path}"))
                    }
                    n => bail!(
                        "tool 'edit': `old_string` appears {n} times in \
                         '{file_path}'; it must be unique. Include more \
                         surrounding context so it matches exactly one \
                         location."
                    ),
                }
            }
        }
        "bash" => {
            let command = arguments["command"]
                .as_str()
                .ok_or_else(|| eyre!("tool 'bash' requires a string field 'command'"))?;
            // kill_on_drop ensures that aborting the tool task (on Esc / app
            // shutdown) terminates the child instead of orphaning it. There is
            // deliberately no per-command timeout: the user cancels with Esc.
            let output = Command::new("bash")
                .arg("-c")
                .arg(command)
                .kill_on_drop(true)
                .output()
                .await?;
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
            // A bounded client with a hard 30s timeout so an unresponsive URL
            // can never stall the agent indefinitely. Dropping the future (on
            // Esc / shutdown) also cancels the request cleanly.
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .user_agent(HTTP_USER_AGENT)
                .build()?;
            let response = client.get(url).send().await?;
            response.text().await.map_err(Into::into)
        }
        _ => Err(eyre!("attempted to call unknown tool '{name}'")),
    }
}
