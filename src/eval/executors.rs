use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::agent::system_prompt::SYSTEM_PROMPT;
use crate::agent::tool_registry::ToolRegistry;
use crate::api::client::OpenAIClient;
use crate::api::types::{ChatCompletionRequest, Message, ToolDefinition};

/// Send a single user message and return the tool name to the model chose.
pub async fn run_single_turn(
    client: &OpenAIClient,
    tools: &[ToolDefinition],
    input: &str,
) -> Result<Option<String>> {
    let request = ChatCompletionRequest {
        model: "mimo-v2.5-free".into(),
        messages: vec![Message::system(SYSTEM_PROMPT), Message::user(input)],
        tools: Some(tools.to_vec()),
        stream: None,
    };

    let response = client.chat_completion(request).await?;

    let tool_name = response
        .choices
        .first()
        .and_then(|c| c.message.tool_calls.as_ref())
        .and_then(|calls| calls.first())
        .map(|tc| tc.function.name.clone());

    Ok(tool_name)
}

/// Run a full agent loop and collect tool calls + final response.
pub async fn run_multi_turn(
    client: &OpenAIClient,
    registry: &ToolRegistry,
    tools: &[ToolDefinition],
    input: &str,
) -> Result<(Vec<String>, String)> {
    let tool_names: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let final_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    let tool_names_clone = Arc::clone(&tool_names);
    let final_text_clone = Arc::clone(&final_text);

    let mut callbacks = crate::agent::run::AgentCallbacks {
        on_token: Box::new(move |token| {
            final_text_clone.lock().unwrap().push_str(token);
        }),
        on_tool_call_start: Box::new(move |name, _args| {
            tool_names_clone.lock().unwrap().push(name.to_string());
        }),
        on_tool_call_end: Box::new(|_, _| {}),
        on_tool_approval: Box::new(|_, _| true),
        on_complete: Box::new(|_| {}),
        on_token_usage: Box::new(|_| {}),
    };

    crate::agent::run::run_agent(input, Vec::new(), client, registry, tools, &mut callbacks)
        .await?;

    let tools_used = tool_names.lock().unwrap().clone();
    let response = final_text.lock().unwrap().clone();

    Ok((tools_used, response))
}
