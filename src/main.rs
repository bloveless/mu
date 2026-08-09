mod agent;
mod api;
mod events;
mod theme;
mod tools;
mod ui;

use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use crossterm::execute;

use crate::agent::run::run_agent;
use crate::agent::tool_registry::ToolRegistry;
use crate::api::client::OpenAIClient;
use crate::events::{AIEvent, AppEvent};
use crate::tools::fetch::FetchTool;
use crate::tools::file::{DeleteFileTool, EditFileTool, ListFilesTool, ReadFileTool};
use crate::tools::shell::RunCommandTool;
use crate::tools::web_search::WebSearchTool;

const DEFAULT_INSTRUCTIONS: &str = include_str!("DEFAULT_INSTRUCTIONS.md");

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    /// The base URL for the OpenAI-compatible API (default: "https://opencode.ai/zen/go/v1/chat/completions").
    #[arg(
        long,
        env = "OPENCODE_BASE_URL",
        default_value = "https://opencode.ai/zen/go/v1/chat/completions"
    )]
    base_url: String,

    /// The model to use for the harness (default: "deepseek-v4-flash").
    #[arg(long, env = "OPENCODE_MODEL", default_value = "deepseek-v4-flash")]
    model: String,

    /// The API key for the OpenAI-compatible API.
    #[arg(long, env = "OPENCODE_API_KEY")]
    api_key: String,

    /// The firecrawl API key.
    #[arg(long, env = "FIRECRAWL_API_KEY")]
    firecrawl_api_key: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    _ = dotenvy::dotenv();

    #[cfg(feature = "console")]
    console_subscriber::init();
    let args = Args::parse();
    let mut set = tokio::task::JoinSet::new();

    let token = CancellationToken::new();
    let (event_tx, event_rx) = std::sync::mpsc::channel::<AppEvent>();
    let (ai_tx, ai_rx) = mpsc::unbounded_channel::<AIEvent>();

    let client = OpenAIClient::new(args.base_url, args.api_key);

    // Build the tool registry
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadFileTool));
    registry.register(Box::new(ListFilesTool));
    registry.register(Box::new(EditFileTool));
    registry.register(Box::new(DeleteFileTool));
    registry.register(Box::new(RunCommandTool));
    registry.register(Box::new(WebSearchTool::new(args.firecrawl_api_key.clone())));
    registry.register(Box::new(FetchTool::new(args.firecrawl_api_key)));

    let agent_token = token.clone();
    let agent_events = event_tx.clone();
    set.spawn(async move {
        match run_agent(
            agent_token,
            client,
            args.model,
            registry,
            agent_events.clone(),
            ai_rx,
        )
        .await
        {
            Ok(_) => {}
            Err(e) => {
                _ = agent_events.send(AppEvent::Fatal(format!("Agent error: {e:#}")));
                return Err(e);
            }
        }

        Ok(())
    });

    let event_tx = event_tx.clone();
    let event_token = token.clone();
    set.spawn_blocking(move || {
        while !event_token.is_cancelled() {
            if let Ok(true) = event::poll(Duration::from_millis(100)) {
                match event::read() {
                    Ok(Event::Key(key)) => {
                        if key.kind == KeyEventKind::Press
                            && event_tx.send(AppEvent::Key(key)).is_err()
                        {
                            break;
                        }
                    }
                    Ok(Event::Paste(text)) => {
                        if event_tx.send(AppEvent::Paste(text)).is_err() {
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

    let shutdown_token = token.clone();
    set.spawn_blocking(move || {
        crossterm::terminal::enable_raw_mode()?;
        execute!(std::io::stdout(), EnableBracketedPaste)?;
        let result = ui::App::new(event_rx, ai_tx).run();
        execute!(std::io::stdout(), DisableBracketedPaste)?;
        crossterm::terminal::disable_raw_mode()?;

        // The UI has quit. Cancel so the agent's `select!` arms fire and the
        // crossterm reader winds down, then join both helper threads.
        token.cancel();

        result
    });

    let mut task_error = None;
    while let Some(result) = set.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                shutdown_token.cancel();
                if task_error.is_none() {
                    task_error = Some(error)
                }
            }
            Err(e) => {
                shutdown_token.cancel();
                if task_error.is_none() {
                    task_error = Some(e.into());
                }
            }
        }
    }

    if let Some(error) = task_error {
        return Err(error);
    }

    Ok(())
}
