use anyhow::Result;
use serde_json::Value;

use crate::agent::system_prompt::SYSTEM_PROMPT;
use crate::agent::tool_registry::ToolRegistry;
use crate::api::client::OpenAIClient;
use crate::api::types::{ChatCompletionRequest, FunctionCall, Message, ToolCall, ToolDefinition};
use crate::context::compaction::compact_conversation;
use crate::context::model_limits::{get_token_usage, should_compact};
use crate::context::token_estimator::estimate_conversation_tokens;

/// Accumulated state for a tool call being streamed.
#[derive(Debug, Clone)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Callbacks for the agent loop.
pub struct AgentCallbacks {
    pub on_token: Box<dyn FnMut(&str)>,
    pub on_tool_call_start: Box<dyn FnMut(&str, &Value)>,
    pub on_tool_call_end: Box<dyn FnMut(&str, &str)>,
    pub on_complete: Box<dyn FnMut(&str)>,
    pub on_token_usage: Box<dyn FnMut(crate::context::model_limits::TokenUsageInfo)>,
}

/// Run the agent loop
pub async fn run_agent(
    user_message: &str,
    history: Vec<Message>,
    client: &OpenAIClient,
    registry: &ToolRegistry,
    tools: &[ToolDefinition],
    callbacks: &mut AgentCallbacks,
) -> Result<Vec<Message>> {
    let mut messages = history;

    // Add system prompt if not present
    if messages.is_empty() || messages[0].role != "system" {
        messages.insert(0, Message::system(SYSTEM_PROMPT));
    }

    // Add the user's message
    messages.push(Message::user(user_message));

    loop {
        // Check context usage
        let token_count = estimate_conversation_tokens(&messages);
        let model = "mimo-v2.5-pro";

        let usage = get_token_usage(token_count, model);
        (callbacks.on_token_usage)(usage);

        if should_compact(token_count, model) {
            messages = compact_conversation(client, &messages).await?;

            // Re-add the latest user message if compaction removed it
            // (The user's most recent message is important context)
        }

        // --- Accumulation state for this iteration ---
        let mut text_content = String::new();
        let mut pending_tools: Vec<PendingToolCall> = Vec::new();
        let mut finish_reason = None;

        // --- Stream the response ---
        let request = ChatCompletionRequest {
            model: "mimo-v2.5-free".to_string(),
            messages: messages.clone(),
            tools: Some(tools.to_vec()),
            stream: Some(true),
        };

        client
            .chat_completion_stream(request, |chunk| {
                if let Some(choice) = chunk.choices.first() {
                    // Capture finish reason
                    if let Some(ref reason) = choice.finish_reason {
                        // TODO: what does `ref reason` mean, why is it necessary
                        finish_reason = Some(reason.clone());
                    }

                    let delta = &choice.delta;

                    // Text content
                    if let Some(ref content) = delta.content {
                        text_content.push_str(content);
                        (callbacks.on_token)(content);
                    }

                    // Tool calls
                    if let Some(ref tool_calls) = delta.tool_calls {
                        for tc in tool_calls {
                            let idx = tc.index;

                            // Ensure we have a slot for this tool call
                            while pending_tools.len() <= idx {
                                pending_tools.push(PendingToolCall {
                                    id: String::new(),
                                    name: String::new(),
                                    arguments: String::new(),
                                });
                            }

                            // Fill in the fields as they arrive
                            if let Some(ref id) = tc.id {
                                pending_tools[idx].id = id.clone();
                            }
                            if let Some(ref func) = tc.function {
                                if let Some(ref name) = func.name {
                                    pending_tools[idx].name = name.clone();
                                }
                                if let Some(ref args) = func.arguments {
                                    pending_tools[idx].arguments.push_str(args);
                                }
                            }
                        }
                    }
                }
            })
            .await?;

        // --- Process the completed response ---

        // If the model just returned the text, we're done
        if finish_reason.as_deref() == Some("stop") || pending_tools.is_empty() {
            // Add assistant message to history
            if !text_content.is_empty() {
                messages.push(Message::assistant(&text_content));
            }
            (callbacks.on_complete)(&text_content);
            return Ok(messages);
        }

        // --- Execute tool calls ---

        // Build the assistant message with tool calls
        let tool_calls: Vec<ToolCall> = pending_tools
            .iter()
            .map(|pt| ToolCall {
                id: pt.id.clone(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: pt.name.clone(),
                    arguments: pt.arguments.clone(),
                },
            })
            .collect();

        messages.push(Message {
            role: "assistant".to_string(),
            content: if text_content.is_empty() {
                None
            } else {
                Some(text_content.clone())
            },
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        });

        // Execute each tool and add results
        for pt in &pending_tools {
            let args: Value = serde_json::from_str(&pt.arguments).unwrap_or(Value::Null);

            (callbacks.on_tool_call_start)(&pt.name, &args);

            let result = registry.execute(&pt.name, args).await?;

            (callbacks.on_tool_call_end)(&pt.name, &result);

            messages.push(Message::tool_result(&pt.id, &result));
        }

        // Loop back - the LLM will see the tool results and continue
    }
}
