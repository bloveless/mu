use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::{
    DEFAULT_INSTRUCTIONS,
    agent::tool_registry::ToolRegistry,
    api::{
        client::OpenAIClient,
        types::{ChatCompletionRequest, FunctionCall, Message, ToolCall},
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
    registry: Arc<ToolRegistry>,
    event_tx: std::sync::mpsc::Sender<AppEvent>,
    ai_rx: tokio::sync::mpsc::UnboundedReceiver<AIEvent>,
) -> Result<()> {
    let fatal_events = event_tx.clone();
    let result = openai_stuff(&token, &client, registry, event_tx, ai_rx).await;
    if let Err(err) = &result {
        // The agent is dying; tell the UI so it can exit with a useful
        // message instead of hanging on the next prompt send.
        let _ = fatal_events.send(AppEvent::Fatal(format!("{err:?}")));
    }
    result
}

async fn openai_stuff(
    token: &CancellationToken,
    client: &OpenAIClient,
    registry: Arc<ToolRegistry>,
    tx_events: std::sync::mpsc::Sender<AppEvent>,
    mut rx_events: tokio::sync::mpsc::UnboundedReceiver<AIEvent>,
) -> Result<()> {
    let mut messages: Vec<Message> = vec![Message::system(DEFAULT_INSTRUCTIONS)];

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

        messages.push(Message::user(&prompt));

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

                let request = ChatCompletionRequest {
                    model: "deepseek-v4-flash".into(),
                    messages: messages.clone(),
                    tools: Some(registry.definitions()),
                    stream: Some(true),
                };

                let mut finish_reason = None;
                let mut assistant_message = String::new();
                let mut pending_tools: Vec<PendingToolCall> = Vec::new();

                let chat_completion_handle = client.chat_completion_stream(request, |chunk| {
                    if let Some(choice) = chunk.choices.first() {
                        if let Some(reason) = &choice.finish_reason {
                            finish_reason = Some(reason.clone());
                        }

                        let delta = &choice.delta;

                        if let Some(reasoning_content) = &delta.reasoning_content {
                            tx_events
                                .send(AppEvent::ThinkingChunkReceived(reasoning_content.clone()))
                                .ok();
                        }

                        if let Some(content) = &delta.content {
                            assistant_message.push_str(content.as_str());
                            tx_events
                                .send(AppEvent::ChunkReceived(content.clone()))
                                .ok();
                        }

                        if let Some(tool_calls) = &delta.tool_calls {
                            for tc in tool_calls {
                                let idx = tc.index;

                                while pending_tools.len() <= idx {
                                    pending_tools.push(PendingToolCall {
                                        id: String::new(),
                                        name: String::new(),
                                        arguments: String::new(),
                                    })
                                }

                                // Fill in the fields as they arrive
                                if let Some(id) = &tc.id {
                                    pending_tools[idx].id = id.clone();
                                }
                                if let Some(func) = &tc.function {
                                    if let Some(name) = &func.name {
                                        pending_tools[idx].name = name.clone();
                                    }
                                    if let Some(args) = &func.arguments {
                                        pending_tools[idx].arguments.push_str(args);
                                    }
                                }
                            }
                        }
                    }
                });

                tokio::select! {
                    result = chat_completion_handle => {
                        if let Err(e) = result {
                            eprintln!("Chat completion stream error: {e}");
                        }
                    },
                    _ = token.cancelled() => break 'turn,
                    _ = turn_token.cancelled() => break 'turn,
                };

                if !assistant_message.is_empty() || !pending_tools.is_empty() {
                    let mut msg = Message::assistant(&assistant_message);
                    let mut tool_calls = vec![];
                    for tc in &pending_tools {
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
                    msg.tool_calls = Some(tool_calls);
                    messages.push(msg);
                }

                let mut tool_set = JoinSet::new();
                for tool_call in pending_tools {
                    let tool_id = tool_call.id.clone();
                    match serde_json::from_str::<Value>(&tool_call.arguments) {
                        Ok(value) => {
                            let tool_name = tool_call.name.clone();
                            let tool_id = tool_call.id.clone();
                            let tool_task_registry = registry.clone();
                            let tx_events = tx_events.clone();
                            tool_set.spawn(async move {
                                _ = tx_events.send(AppEvent::ToolCallStart {
                                    name: tool_name.clone(),
                                    args: value.to_string(),
                                });
                                let result: Result<String> = tool_task_registry
                                    .execute(&tool_name.clone(), value.clone())
                                    .await;
                                match &result {
                                    Ok(r) => {
                                        _ = tx_events.send(AppEvent::ToolCallOutput {
                                            name: tool_name,
                                            output: r.clone(),
                                            success: true,
                                        });
                                    }
                                    Err(e) => {
                                        _ = tx_events.send(AppEvent::ToolCallOutput {
                                            name: tool_name,
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
                            });
                        }
                        Err(e) => messages.push(Message::tool_result(
                            &tool_id,
                            &format!("tool arguments were not valid JSON: {e}"),
                        )),
                    };
                }

                let cancelled = loop {
                    let next = tokio::select! {
                        result = tool_set.join_next() => result,
                        _ = token.cancelled() => break 'turn,
                        _ = turn_token.cancelled() => break 'turn,
                    };
                    match next {
                        Some(Ok(tool_message)) => messages.push(tool_message),
                        Some(Err(e)) => eprintln!("Chat completion stream error: {e}"),
                        None => break false,
                    }
                };
                if cancelled {
                    break 'turn;
                }

                if finish_reason.as_deref() == Some("stop")
                    || finish_reason.as_deref() == Some("length")
                {
                    return Ok(());
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
}
