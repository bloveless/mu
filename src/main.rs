mod agent;
mod api;
mod events;
mod theme;
mod tools;
mod ui;
mod wrap;

use std::sync::Arc;
use std::{env, process, time::Duration};

use anyhow::Result;
use clap::Parser;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crossterm::event::{self, Event, KeyEventKind};

use crate::agent::run::run_agent;
use crate::agent::tool_registry::ToolRegistry;
use crate::api::client::OpenAIClient;
use crate::events::{AIEvent, AppEvent};
use crate::tools::fetch::FetchTool;
use crate::tools::file::{DeleteFileTool, ListFilesTool, ReadFileTool, WriteFileTool};
use crate::tools::shell::{CodeExecutionTool, RunCommandTool};
use crate::tools::web_search::WebSearchTool;

const DEFAULT_INSTRUCTIONS: &str = include_str!("DEFAULT_INSTRUCTIONS.md");

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    /// Run in JSON-RPC server mode instead of TUI mode.
    #[arg(long)]
    json: bool,

    /// Port for the JSON-RPC server (default: 3000).
    #[arg(long, default_value = "3000")]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv()?;

    #[cfg(feature = "console")]
    console_subscriber::init();
    let _args = Args::parse();
    let mut set = tokio::task::JoinSet::new();

    let token = CancellationToken::new();
    let (event_tx, event_rx) = std::sync::mpsc::channel::<AppEvent>();
    let (ai_tx, ai_rx) = mpsc::unbounded_channel::<AIEvent>();

    let base_url = env::var("OPENCODE_BASE_URL")
        .unwrap_or_else(|_| "https://opencode.ai/zen/go/v1/chat/completions".to_string());
    let api_key = env::var("OPENCODE_API_KEY").unwrap_or_else(|_| {
        eprintln!("OPENCODE_API_KEY is not set");
        process::exit(1);
    });
    let firecrawl_api_key =
        std::env::var("FIRECRAWL_API_KEY").expect("FIRECRAWL_API_KEY must be set");

    let client = OpenAIClient::new(base_url, api_key);

    // Build the tool registry
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadFileTool));
    registry.register(Box::new(ListFilesTool));
    registry.register(Box::new(WriteFileTool));
    registry.register(Box::new(DeleteFileTool));
    registry.register(Box::new(RunCommandTool));
    registry.register(Box::new(CodeExecutionTool));
    registry.register(Box::new(WebSearchTool::new(firecrawl_api_key.clone())));
    registry.register(Box::new(FetchTool::new(firecrawl_api_key)));

    let agent_token = token.clone();
    let agent_events = event_tx.clone();
    set.spawn(run_agent(
        agent_token,
        client,
        Arc::new(registry),
        agent_events,
        ai_rx,
    ));


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

    set.spawn_blocking(move || {
        let mut terminal = ratatui::init();
        if let Err(e) = ui::App::new(event_rx, ai_tx).run(&mut terminal) {
            eprintln!("UI error: {:?}", e);
        }
        ratatui::restore();

        // The UI has quit. Cancel so the agent's `select!` arms fire and the
        // crossterm reader winds down, then join both helper threads.
        token.cancel();

        Ok(())
    });

    while let Some(result) = set.join_next().await {
        match result {
            Ok(output) => println!("Task finished: {:?}", output),
            Err(e) => eprintln!("Task error: {:?}", e),
        }
    }

    Ok(())
}
