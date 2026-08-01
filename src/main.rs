mod agent;
mod api;
mod context;
mod eval;
mod tools;
mod ui;

use std::sync::{Arc, Mutex};

use anyhow::Result;
use api::client::OpenAIClient;

use crate::{
    agent::{
        run::{AgentCallbacks, run_agent},
        tool_registry::ToolRegistry,
    },
    tools::{
        fetch::FetchTool,
        file::{DeleteFileTool, WriteFileTool},
        shell::{CodeExecutionTool, RunCommandTool},
        web_search::WebSearchTool,
    },
    ui::{app::AppState, bridge::drive_agent},
};
use tools::file::{ListFilesTool, ReadFileTool};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv()?;

    let api_key = std::env::var("OPENCODE_API_KEY").expect("OPENCODE_API_KEY must be set");
    let firecrawl_api_key =
        std::env::var("FIRECRAWL_API_KEY").expect("FIRECRAWL_API_KEY must be set");

    let client = Arc::new(OpenAIClient::new(api_key));

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

    let registry = Arc::new(registry);
    let definitions = Arc::new(registry.definitions());

    let state = Arc::new(Mutex::new(AppState::new()));

    // Background tokio task: polls state for submissions, drives the agent loop
    let agent_task = tokio::spawn(drive_agent(
        Arc::clone(&state),
        Arc::clone(&client),
        Arc::clone(&registry),
        Arc::clone(&definitions),
        Vec::new(),
    ));

    // Run the UI on the main thread
    let ui_state = Arc::clone(&state);
    let ui_task = tokio::task::spawn_blocking(move || ui::event_loop::run_ui(ui_state));

    // Wait for both tasks to complete
    tokio::join!(agent_task, ui_task);

    Ok(())
}
