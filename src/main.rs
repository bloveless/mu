mod agent;
mod api;
mod context;
mod eval;
mod tools;

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
};
use tools::file::{ListFilesTool, ReadFileTool};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv()?;

    let api_key = std::env::var("OPENCODE_API_KEY").expect("OPENCODE_API_KEY must be set");
    let firecrawl_api_key =
        std::env::var("FIRECRAWL_API_KEY").expect("FIRECRAWL_API_KEY must be set");

    let client = OpenAIClient::new(api_key);

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

    let definitions = registry.definitions();

    let mut callbacks = AgentCallbacks {
        on_token: Box::new(|token| print!("{token}")),
        on_tool_call_start: Box::new(|name, _args| println!("\n[calling: {name}...]")),
        on_tool_call_end: Box::new(|name, result| {
            let preview = &result[..result.len().min(100)];
            println!("[{name} done: {preview}");
        }),
        on_complete: Box::new(|_| {
            println!();
        }),
        on_token_usage: Box::new(|_| {}),
    };

    let messages = run_agent(
        // "Create a file called test.txt with ‘Hello from the agent’, then read it back to verify.",
        "Search the web for rust raylib and give me some suggestions.",
        // "Fetch the content of https://crates.io/crates/raylib and give me an overview of what it does.",
        Vec::new(),
        &client,
        &registry,
        &definitions,
        &mut callbacks,
    )
    .await?;

    println!("\n--- Conversation: {} messages ---", messages.len());

    Ok(())
}
