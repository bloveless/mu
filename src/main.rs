mod agent;
mod api;
mod eval;
mod tools;

use anyhow::Result;
use api::client::OpenAIClient;

use crate::{
    agent::{
        run::{AgentCallbacks, run_agent},
        tool_registry::ToolRegistry,
    },
    tools::file::{DeleteFileTool, WriteFileTool},
};
use tools::file::{ListFilesTool, ReadFileTool};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv()?;

    let api_key = std::env::var("OPENCODE_API_KEY").expect("OPENCODE_API_KEY must be set");

    let client = OpenAIClient::new(api_key);

    // Build the tool registry
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadFileTool));
    registry.register(Box::new(ListFilesTool));
    registry.register(Box::new(WriteFileTool));
    registry.register(Box::new(DeleteFileTool));

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
    };

    let messages = run_agent(
        "Create a file called test.txt with ‘Hello from the agent’, then read it back to verify.",
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
