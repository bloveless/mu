//! JSON-RPC 2.0 HTTP frontend for `mu`.
//!
//! Replaces the ratatui TUI with an HTTP server that speaks a simple
//! JSON-RPC 2.0 protocol.  An LLM agent (or any HTTP client) can submit
//! prompts, poll for events (assistant responses, tool calls, errors), cancel
//! the in-flight turn, or shut down the server.
//!
//! # Protocol
//!
//! Every request is a `POST /` with a JSON-RPC 2.0 body:
//!
//! ```json
//! {"jsonrpc": "2.0", "id": 1, "method": "submit_prompt", "params": {"prompt": "..."}}
//! ```
//!
//! ## Methods
//!
//! | Method           | Params                          | Returns                                          |
//! |------------------|---------------------------------|--------------------------------------------------|
//! | `submit_prompt`  | `{"prompt": "..."}`             | `{"accepted": true}`                             |
//! | `get_events`     | `{"since_id": 0}`               | `{"events": […], "next_id": N}`                  |
//! | `cancel_turn`    | `{}`                            | `{"success": true}`                              |
//! | `shutdown`       | `{}`                            | `{"shutting_down": true}`                        |
//! | `get_status`     | `{}`                            | `{"working": false}`                             |

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::{AIEvent, AppEvent};

// ---------------------------------------------------------------------------
// Event store — thread-safe, append-only log of AppEvents
// ---------------------------------------------------------------------------

/// A single recorded event.
#[derive(Clone, Serialize)]
struct StoredEvent {
    id: u64,
    #[serde(rename = "type")]
    event_type: String,
    data: serde_json::Value,
    timestamp: SystemTime,
}

/// Append-only log of [`AppEvent`]s that HTTP handlers query.
///
/// The background collector drains the agent's event channel and pushes into
/// this store; `get_events` reads from it so callers can poll incrementally.
#[derive(Clone)]
struct EventStore {
    events: Arc<Mutex<Vec<StoredEvent>>>,
    next_id: Arc<AtomicU64>,
}

impl EventStore {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    fn push(&self, event: AppEvent) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (event_type, data) = Self::classify(&event);
        let stored = StoredEvent { id, event_type, data, timestamp: SystemTime::now() };
        self.events.lock().expect("EventStore lock poisoned").push(stored);
    }

    /// Return all events whose `id > since_id`.
    fn since(&self, since_id: u64) -> Vec<StoredEvent> {
        let guard = self.events.lock().expect("EventStore lock poisoned");
        let start = guard.partition_point(|e| e.id <= since_id);
        guard[start..].to_vec()
    }

    fn classify(event: &AppEvent) -> (String, serde_json::Value) {
        match event {
            AppEvent::AssistantResponse(text) => {
                ("assistant_response".into(), serde_json::json!({"content": text}))
            }
            AppEvent::Error(msg) => {
                ("error".into(), serde_json::json!({"message": msg}))
            }
            AppEvent::ToolCallStart { name, args } => {
                ("tool_call_start".into(), serde_json::json!({"name": name, "arguments": args}))
            }
            AppEvent::ToolCallOutput { name, output, success } => {
                ("tool_call_output".into(), serde_json::json!({"name": name, "output": output, "success": success}))
            }
            AppEvent::TurnEnd => ("turn_end".into(), serde_json::Value::Null),
            AppEvent::Fatal(msg) => {
                ("fatal".into(), serde_json::json!({"message": msg}))
            }
            // TUI-specific; never appears in JSON mode
            #[cfg(feature = "tui")]
            AppEvent::Key(_) | AppEvent::Resize => {
                ("ignored".into(), serde_json::Value::Null)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 wire types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[serde(default = "default_jsonrpc")]
    #[allow(dead_code)]
    jsonrpc: String,
    #[serde(default)]
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

fn default_jsonrpc() -> String {
    "2.0".into()
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

fn ok_response(id: Option<serde_json::Value>, result: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
}

fn err_response(
    id: Option<serde_json::Value>,
    code: i32,
    message: impl Into<String>,
) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(JsonRpcError { code, message: message.into(), data: None }),
    }
}

// ---------------------------------------------------------------------------
// Axum application state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AppState {
    ai_tx: UnboundedSender<AIEvent>,
    store: EventStore,
    token: CancellationToken,
    current_turn: Arc<Mutex<Option<CancellationToken>>>,
}

// ---------------------------------------------------------------------------
// Public entry-point
// ---------------------------------------------------------------------------

/// Run the JSON-RPC frontend: spawn an event collector and an HTTP server,
/// then block until shutdown (agent death, fatal error, or `shutdown` RPC).
pub fn run_json_frontend(
    event_rx: std::sync::mpsc::Receiver<AppEvent>,
    ai_tx: UnboundedSender<AIEvent>,
    token: CancellationToken,
    port: u16,
) -> Result<(), color_eyre::Report> {
    let store = EventStore::new();
    let current_turn: Arc<Mutex<Option<CancellationToken>>> =
        Arc::new(Mutex::new(None));

    // ---- Event collector ----
    //
    // Drains AppEvents from the channel (same one the agent writes to) and
    // pushes them into the EventStore.  When a turn ends (naturally or by
    // cancellation) or a fatal error occurs, it clears the current-turn
    // token so `get_status` accurately reflects activity.
    let collector_store = store.clone();
    let collector_token = token.clone();
    let collector_turn = current_turn.clone();
    std::thread::Builder::new()
        .name("event-collector".into())
        .spawn(move || loop {
            match event_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => {
                    if matches!(&event, AppEvent::Fatal(_)) {
                        collector_token.cancel();
                    }
                    if matches!(&event, AppEvent::TurnEnd) {
                        collector_turn.lock().expect("lock poisoned").take();
                    }
                    collector_store.push(event);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if collector_token.is_cancelled() {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    // Agent has gone away (panicked or all senders dropped).
                    collector_token.cancel();
                    break;
                }
            }
        })?;

    // ---- HTTP server ----
    let server_token = token.clone();
    let server_turn = current_turn.clone();
    let server_store = store.clone();

    let server_handle = std::thread::Builder::new()
        .name("json-rpc-server".into())
        .spawn(move || -> Result<(), color_eyre::Report> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;

            runtime.block_on(async move {
                let state = AppState {
                    ai_tx,
                    store: server_store,
                    token: server_token.clone(),
                    current_turn: server_turn,
                };

                let app =
                    Router::new().route("/", post(json_rpc_handler)).with_state(state);

                let addr = format!("0.0.0.0:{port}");
                let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| {
                    color_eyre::eyre::eyre!("Failed to bind JSON-RPC server to {addr}: {e}")
                })?;

                eprintln!("JSON-RPC server listening on http://{addr}");

                axum::serve(listener, app)
                    .with_graceful_shutdown(async move {
                        server_token.cancelled().await;
                    })
                    .await
                    .map_err(|e| color_eyre::eyre::eyre!("JSON-RPC server error: {e}"))?;

                Ok(())
            })
        })?;

    // Block until the server thread finishes.
    match server_handle.join() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(color_eyre::eyre::eyre!("JSON-RPC server thread panicked")),
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC method dispatcher
// ---------------------------------------------------------------------------

async fn json_rpc_handler(
    State(state): State<AppState>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    let id = req.id.clone();
    match req.method.as_str() {
        "submit_prompt" => handle_submit_prompt(&state, req.params, id).await,
        "get_events" => handle_get_events(&state, req.params, id),
        "cancel_turn" => handle_cancel_turn(&state, id),
        "shutdown" => handle_shutdown(&state, id),
        "get_status" => handle_get_status(&state, id),
        _ => Json(err_response(id, -32601, "Method not found")),
    }
}

// ---------------------------------------------------------------------------
// Method implementations
// ---------------------------------------------------------------------------

/// Submit a user prompt to the agent.
///
/// The agent processes asynchronously; poll `get_events` for results.
async fn handle_submit_prompt(
    state: &AppState,
    params: Option<serde_json::Value>,
    id: Option<serde_json::Value>,
) -> Json<JsonRpcResponse> {
    let prompt = match params
        .as_ref()
        .and_then(|p| p.get("prompt")?.as_str().map(String::from))
    {
        Some(p) => p,
        None => {
            return Json(err_response(id, -32602, "Missing required parameter 'prompt' (string)"));
        }
    };

    let turn_token = CancellationToken::new();
    *state.current_turn.lock().expect("lock poisoned") = Some(turn_token.clone());

    if state.ai_tx.send(AIEvent::UserPrompt(prompt, turn_token)).is_err() {
        return Json(err_response(id, -32000, "Agent task is no longer running"));
    }

    Json(ok_response(id, serde_json::json!({"accepted": true})))
}

/// Return events since the given ID.
///
/// The caller should track the highest `id` received and use it as
/// `since_id` in the next poll.
fn handle_get_events(
    state: &AppState,
    params: Option<serde_json::Value>,
    id: Option<serde_json::Value>,
) -> Json<JsonRpcResponse> {
    let since_id = params
        .as_ref()
        .and_then(|p| p.get("since_id").and_then(|v| v.as_u64()))
        .unwrap_or(0);

    let events = state.store.since(since_id);
    let next_id = events.last().map(|e| e.id).unwrap_or(since_id);

    Json(ok_response(id, serde_json::json!({"events": events, "next_id": next_id})))
}

/// Cancel the currently running turn.
fn handle_cancel_turn(
    state: &AppState,
    id: Option<serde_json::Value>,
) -> Json<JsonRpcResponse> {
    let prev = state.current_turn.lock().expect("lock poisoned").take();
    match prev {
        Some(token) => {
            token.cancel();
            Json(ok_response(id, serde_json::json!({"success": true})))
        }
        None => Json(ok_response(
            id,
            serde_json::json!({"success": false, "reason": "No active turn"}),
        )),
    }
}

/// Gracefully shut down the server.
fn handle_shutdown(
    state: &AppState,
    id: Option<serde_json::Value>,
) -> Json<JsonRpcResponse> {
    state.token.cancel();
    Json(ok_response(id, serde_json::json!({"shutting_down": true})))
}

/// Return status information about the agent.
fn handle_get_status(
    state: &AppState,
    id: Option<serde_json::Value>,
) -> Json<JsonRpcResponse> {
    let working = state.current_turn.lock().expect("lock poisoned").is_some();
    let event_count = state.store.events.lock().expect("lock poisoned").len();
    Json(ok_response(
        id,
        serde_json::json!({"working": working, "event_count": event_count}),
    ))
}
