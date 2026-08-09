//! History item model. Rollout step 1 keeps an empty history; the streaming
//! state machine and truncation land in steps 2–4.
#![allow(dead_code)]

use crate::ToolCallId;

/// One entry in the scrollback history.
#[derive(Debug)]
pub enum HistoryItem {
    Thinking(String),
    Message(String),
    ToolCall(ToolCallItem),
    Error(String),
}

#[derive(Debug)]
pub struct ToolCallItem {
    pub id: ToolCallId,
    pub name: String,
    pub args: String,
    pub output: String,
    pub status: ToolCallStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallStatus {
    Running,
    Success,
    Failed,
}

/// Ordered scrollback of history items.
#[derive(Debug, Default)]
pub struct History {
    items: Vec<HistoryItem>,
}

impl History {
    pub fn push(&mut self, item: HistoryItem) {
        self.items.push(item);
    }

    pub fn items(&self) -> &[HistoryItem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
