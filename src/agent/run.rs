use anyhow::Result;
use tokio_util::sync::CancellationToken;

use crate::{
    DEFAULT_INSTRUCTIONS,
    agent::tool_registry::ToolRegistry,
    api::{
        client::OpenAIClient,
        types::{ChatCompletionRequest, Message},
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
    registry: ToolRegistry,
    event_tx: std::sync::mpsc::Sender<AppEvent>,
    ai_rx: tokio::sync::mpsc::UnboundedReceiver<AIEvent>,
) -> Result<()> {
    // Clone for the post-agent `Fatal` send; `event_tx` itself moves into
    // `openai_stuff` so tool tasks can stream results.
    let fatal_events = event_tx.clone();
    let result = openai_stuff(&token, &client, &registry, event_tx, ai_rx).await;
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
    registry: &ToolRegistry,
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
                    _ = chat_completion_handle => {},
                    _ = token.cancelled() => break 'turn,
                    _ = turn_token.cancelled() => break 'turn,
                };

                if finish_reason.as_deref() == Some("stop") {
                    return Ok(());
                }

                // let message = choice.message.clone();

                // if message.content.is_some() || message.tool_calls.is_some() {
                //     let mut msg = ChatCompletionRequestAssistantMessageArgs::default();
                //     if let Some(tool_calls) = message.tool_calls.clone() {
                //         msg.tool_calls(tool_calls);
                //     }
                //     if let Some(content) = message.content.clone() {
                //         msg.content(content);
                //     }
                //     messages.push(msg.build()?.into());
                // }

                // if let Some(content) = &message.content
                //     && !content.is_empty()
                // {
                //     let _ = tx_events.send(AppEvent::AssistantResponse(content.clone()));
                // }

                // match choice.finish_reason {
                //     Some(FinishReason::Stop) | Some(FinishReason::Length) => {
                //         break;
                //     }
                //     _ => {}
                // }

                // let Some(tool_calls) = message.tool_calls else {
                //     break;
                // };

                // Tool calls run on a dedicated JoinSet so the whole batch can
                // be aborted atomically when the app or the current turn is
                // cancelled. Aborting drops the futures, which cancels in-flight
                // `fetch` requests and (via `kill_on_drop`) terminates running
                // `bash` children.
                // let mut tool_set: JoinSet<ChatCompletionRequestMessage> = JoinSet::new();
                // for tool_call_enum in tool_calls {
                //     // Extract the function tool call from the enum
                //     if let ChatCompletionMessageToolCalls::Function(tool_call) = tool_call_enum {
                //         let id = tool_call.id.clone();
                //         let name = tool_call.function.name.clone();
                //         let args = tool_call.function.arguments.clone();

                //         let _ = tx_events.send(AppEvent::ToolCallStart {
                //             name: name.clone(),
                //             args: args.clone(),
                //         });

                //         let tx_events = tx_events.clone();
                //         tool_set.spawn(async move {
                //             let result: Result<String> = call_fn(&name, &args).await;
                //             let output = match &result {
                //                 Ok(output) => output.clone(),
                //                 Err(err) => err.to_string(),
                //             };
                //             let success = result.is_ok();
                //             let _ = tx_events.send(AppEvent::ToolCallOutput {
                //                 name: name.clone(),
                //                 output: output.clone(),
                //                 success,
                //             });
                //             ChatCompletionRequestToolMessage {
                //                 content: output.into(),
                //                 tool_call_id: id,
                //             }
                //             .into()
                //         });
                //     }
                // }

                // Drive the tool batch to completion, but abort everything the
                // instant it is cancelled.
                // let cancelled = loop {
                //     let next = tokio::select! {
                //         n = tool_set.join_next() => n,
                //         _ = turn_token.cancelled() => { tool_set.abort_all(); break true; },
                //         _ = token.cancelled() => { tool_set.abort_all(); break true; },
                //     };
                //     match next {
                //         Some(Ok(tool_message)) => messages.push(tool_message),
                //         // A panicked or aborted tool task: nothing to append;
                //         // errors were already surfaced as ToolCallOutput.
                //         Some(Err(_)) => {}
                //         None => break false,
                //     }
                // };
                // if cancelled {
                //     break 'turn;
                // }
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
