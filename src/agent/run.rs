use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{
    DEFAULT_INSTRUCTIONS,
    agent::tool_registry::ToolRegistry,
    api::{
        client::OpenAIClient,
        types::{ChatCompletionRequest, FunctionCall, Message, StreamOptions, ToolCall},
    },
    events::{AIEvent, AppEvent},
};

/// Accumulated state for a tool call being streamed.
#[derive(Debug, Clone)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Runs entirely inside the tokio runtime on the "agent-runtime" thread.
///
/// Spawn the agent loop (`openai_stuff`) as the sole root task; when it
/// returns on cancel or `ai_rx` disconnect, surface any error as a `Fatal`
/// event to the UI. There is no UI-completion coordination here — `main`
/// (the UI) is the one that knows when the app is done, and it cancels the
/// shared token to wind this side down.
pub async fn run_agent(
    token: CancellationToken,
    client: OpenAIClient,
    model: String,
    registry: ToolRegistry,
    app_events: std::sync::mpsc::Sender<AppEvent>,
    mut ai_events: tokio::sync::mpsc::UnboundedReceiver<AIEvent>,
) -> Result<()> {
    let mut messages: Vec<Message> = vec![Message::system(DEFAULT_INSTRUCTIONS)];

    loop {
        // Wait for the next user prompt, but bail out immediately if the app
        // is shutting down. The UI also drops `ai_tx` on quit, which drives
        // `recv` to `None` as a backup path.
        let (prompt, turn_token) = tokio::select! {
            ev = ai_events.recv() => match ev {
                Some(AIEvent::UserPrompt(p, t)) => (p, t),
                None => return Ok(()),
            },
            _ = token.cancelled() => return Ok(()),
        };

        messages.push(Message::user(&prompt));

        // Remember where the turn began so a cancelled turn can be rolled back
        // out of the conversation history. A half-finished tool_call sequence
        // with no matching tool results would otherwise make the next API
        // request fail validation.
        let checkpoint = messages.len();

        'turn: for _i in 0..20 {
            if token.is_cancelled() || turn_token.is_cancelled() {
                break 'turn;
            }

            let request = ChatCompletionRequest {
                model: model.clone(),
                messages: messages.clone(),
                tools: Some(registry.definitions()),
                stream: Some(true),
                stream_options: Some(StreamOptions {
                    include_usage: true,
                }),
            };

            let mut finish_reason = None;
            let mut assistant_message = String::new();
            let mut pending_tools: BTreeMap<usize, PendingToolCall> = BTreeMap::new();

            let chat_completion_handle = client.chat_completion_stream(request, |chunk| {
                if let Some(choice) = chunk.choices.first() {
                    if let Some(reason) = &choice.finish_reason {
                        finish_reason = Some(reason.clone());
                    }

                    let delta = &choice.delta;

                    if let Some(reasoning_content) = &delta.reasoning_content {
                        app_events
                            .send(AppEvent::ThinkingChunkReceived(reasoning_content.clone()))
                            .ok();
                    }

                    if let Some(content) = &delta.content {
                        assistant_message.push_str(content.as_str());
                        app_events
                            .send(AppEvent::ChunkReceived(content.clone()))
                            .ok();
                    }

                    if let Some(tool_calls) = &delta.tool_calls {
                        for tc in tool_calls {
                            let pending_tool =
                                pending_tools.entry(tc.index).or_insert(PendingToolCall {
                                    id: String::new(),
                                    name: String::new(),
                                    arguments: String::new(),
                                });

                            // Fill in the fields as they arrive
                            if let Some(id) = &tc.id {
                                pending_tool.id = id.clone();
                            }
                            if let Some(func) = &tc.function {
                                if let Some(name) = &func.name {
                                    pending_tool.name = name.clone();
                                }
                                if let Some(args) = &func.arguments {
                                    pending_tool.arguments.push_str(args);
                                }
                            }
                        }
                    }
                }

                if let Some(usage) = &chunk.usage {
                    app_events
                        .send(AppEvent::UsageReceived {
                            prompt_tokens: usage.prompt_tokens,
                            completion_tokens: usage.completion_tokens,
                            total_tokens: usage.total_tokens,
                        })
                        .ok();
                }
            });

            tokio::select! {
                result = chat_completion_handle => result?,
                _ = token.cancelled() => break 'turn,
                _ = turn_token.cancelled() => break 'turn,
            };

            if !assistant_message.is_empty() || !pending_tools.is_empty() {
                let mut msg = Message::assistant(&assistant_message);
                let mut tool_calls = vec![];
                for tc in pending_tools.values() {
                    let tool_call = ToolCall {
                        id: tc.id.clone(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: tc.name.clone(),
                            arguments: tc.arguments.clone(),
                        },
                    };
                    tool_calls.push(tool_call);
                }
                msg.tool_calls = if !tool_calls.is_empty() {
                    Some(tool_calls)
                } else {
                    None
                };
                messages.push(msg);
            }

            for tool_call in pending_tools.values() {
                let tool_id = tool_call.id.clone();
                let message = match serde_json::from_str::<Value>(&tool_call.arguments) {
                    Ok(value) => {
                        _ = app_events.send(AppEvent::ToolCallStart {
                            name: tool_call.name.clone(),
                            args: value.to_string(),
                        });
                        let result: Result<String> = registry.execute(&tool_call.name, value).await;
                        match &result {
                            Ok(r) => {
                                _ = app_events.send(AppEvent::ToolCallOutput {
                                    name: tool_call.name.clone(),
                                    output: r.clone(),
                                    success: true,
                                });
                            }
                            Err(e) => {
                                _ = app_events.send(AppEvent::ToolCallOutput {
                                    name: tool_call.name.clone(),
                                    output: e.to_string(),
                                    success: false,
                                });
                            }
                        }
                        match result {
                            Ok(r) => Message::tool_result(&tool_id, &r),
                            Err(e) => Message::tool_result(
                                &tool_call.id,
                                &format!("failed to execute tool call {e}"),
                            ),
                        }
                    }
                    Err(e) => Message::tool_result(
                        &tool_id,
                        &format!("tool arguments were not valid JSON: {e}"),
                    ),
                };
                messages.push(message);
            }

            if finish_reason.as_deref() == Some("stop")
                || finish_reason.as_deref() == Some("length")
            {
                break 'turn;
            }
        }

        let _ = app_events.send(AppEvent::TurnEnd);

        if turn_token.is_cancelled() {
            // Roll back any half-finished assistant/tool messages from this
            // cancelled turn so the next prompt starts from a clean state.
            messages.truncate(checkpoint);
        }
    }
}
