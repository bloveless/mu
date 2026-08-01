use anyhow::Result;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

use crate::agent::run::{AgentCallbacks, run_agent};
use crate::agent::tool_registry::ToolRegistry;
use crate::api::client::OpenAIClient;
use crate::api::types::ToolDefinition;

use super::app::{ActiveTool, AppState, ApprovalRequest, DisplayMessage, ToolStatus};

/// Polls shared state for a queued user submission and drives the agent loop.
/// Runs as a background tokio task for the lifetime of the program.
pub async fn drive_agent(
    state: Arc<Mutex<AppState>>,
    client: Arc<OpenAIClient>,
    registry: Arc<ToolRegistry>,
    tools: Arc<Vec<ToolDefinition>>,
    mut history: Vec<crate::api::types::Message>,
) {
    loop {
        let (submitted, exit) = {
            let mut s = state.lock().unwrap();
            (s.pending_submit.take(), s.should_exit)
        };

        if exit {
            break;
        }

        if let Some(input) = submitted {
            match run_agent_with_ui(
                input,
                history.clone(),
                &client,
                &registry,
                &tools,
                Arc::clone(&state),
            )
            .await
            {
                Ok(updated_history) => history = updated_history,
                Err(e) => {
                    let mut s = state.lock().unwrap();
                    s.messages.push(DisplayMessage {
                        role: "system".into(),
                        content: format!("Error: {e}"),
                    });
                    s.loading = false;
                }
            }
        }

        sleep(Duration::from_millis(50)).await;
    }
}

/// Run the agent on a background tokio task, updating shared state
pub async fn run_agent_with_ui(
    input: String,
    history: Vec<crate::api::types::Message>,
    client: &OpenAIClient,
    registry: &ToolRegistry,
    tools: &[ToolDefinition],
    state: Arc<Mutex<AppState>>,
) -> Result<Vec<crate::api::types::Message>> {
    let state_token = Arc::clone(&state);
    let state_tool_start = Arc::clone(&state);
    let state_tool_end = Arc::clone(&state);
    let state_complete = Arc::clone(&state);
    let state_usage = Arc::clone(&state);
    let state_approval = Arc::clone(&state);

    let mut callbacks = AgentCallbacks {
        on_token: Box::new(move |token| {
            let mut s = state_token.lock().unwrap();
            s.streaming_text.push_str(token);
        }),
        on_tool_call_start: Box::new(move |name, args| {
            let mut s = state_tool_start.lock().unwrap();
            s.active_tool = Some(ActiveTool {
                name: name.to_string(),
                status: ToolStatus::Running,
            });
        }),
        on_tool_call_end: Box::new(move |name, result| {
            let mut s = state_tool_end.lock().unwrap();
            s.active_tool = Some(ActiveTool {
                name: name.to_string(),
                status: ToolStatus::Complete(result.to_string()),
            });
        }),
        on_tool_approval: Box::new(move |name, args| {
            let response = Arc::new(Mutex::new(None));
            let response_clone = Arc::clone(&response);

            {
                let mut s = state_approval.lock().unwrap();
                s.pending_approval = Some(ApprovalRequest {
                    tool_name: name.to_string(),
                    args_preview: serde_json::to_string_pretty(args).unwrap_or_default(),
                    response: response_clone,
                });
            }

            // Wait for user response
            loop {
                std::thread::sleep(std::time::Duration::from_millis(50));
                if let Some(answer) = *response.lock().unwrap() {
                    return answer;
                }
            }
        }),
        on_complete: Box::new(move |text| {
            let mut s = state_complete.lock().unwrap();
            if !s.streaming_text.is_empty() {
                let content = s.streaming_text.clone();
                s.messages.push(DisplayMessage {
                    role: "assistant".into(),
                    content,
                });
                s.streaming_text.clear();
            }
            s.active_tool = None;
            s.loading = false;
        }),
        on_token_usage: Box::new(move |usage| {
            let mut s = state_usage.lock().unwrap();
            s.token_usage = Some(usage);
        }),
    };

    run_agent(&input, history, client, registry, tools, &mut callbacks).await
}
