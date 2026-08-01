use crate::context::model_limits::TokenUsageInfo;
use std::sync::{Arc, Mutex};

// The full UI state
pub struct AppState {
    /// Chat messages to display.
    pub messages: Vec<DisplayMessage>,
    /// Current user input.
    pub input: String,
    /// Cursor position in the input.
    pub cursor: usize,
    /// Whether the agent is processing.
    pub loading: bool,
    /// Current streaming text (not yet committed to messages).
    pub streaming_text: String,
    /// Active tool calls being displayed.
    pub active_tool: Option<ActiveTool>,
    /// Pending approoval request.
    pub pending_approval: Option<ApprovalRequest>,
    /// Pending submit.
    pub pending_submit: Option<String>,
    /// Token usage info.
    pub token_usage: Option<TokenUsageInfo>,
    /// Whether the app should exit.
    pub should_exit: bool,
    /// Scroll offset for the message list.
    pub scroll_offset: u16,
}

#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ActiveTool {
    pub name: String,
    pub status: ToolStatus,
}

#[derive(Debug, Clone)]
pub enum ToolStatus {
    Running,
    Complete(String), // result preview
}

#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub tool_name: String,
    pub args_preview: String,
    pub response: Arc<Mutex<Option<bool>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor: 0,
            loading: false,
            streaming_text: String::new(),
            active_tool: None,
            pending_approval: None,
            pending_submit: None,
            token_usage: None,
            should_exit: false,
            scroll_offset: 0,
        }
    }
}
