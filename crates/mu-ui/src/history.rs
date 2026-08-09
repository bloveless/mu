//! History item model: the streaming state machine (`*Start`/`*Delta`/
//! `*End` append to and seal items) and per-item wrap-height caching for the
//! flattened-line scroll model.

use ratatui::{
    style::Style,
    text::Line,
    widgets::{Paragraph, Wrap},
};

use crate::{ToolCallId, theme::Theme};

/// One entry in the scrollback history.
#[derive(Debug)]
pub enum HistoryItem {
    UserPrompt(String),
    #[allow(dead_code)] // rollout step 4
    Thinking(String),
    Message(String),
    #[allow(dead_code)] // rollout step 3
    ToolCall(ToolCallItem),
    #[allow(dead_code)] // rollout step 3
    Error(String),
}

#[derive(Debug)]
pub struct ToolCallItem {
    #[allow(dead_code)] // rollout step 3
    pub id: ToolCallId,
    #[allow(dead_code)] // rollout step 3
    pub name: String,
    pub args: String,
    #[allow(dead_code)] // rollout step 3
    pub output: String,
    pub status: ToolCallStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallStatus {
    #[allow(dead_code)] // rollout step 3
    Running,
    #[allow(dead_code)] // rollout step 3
    Success,
    #[allow(dead_code)] // rollout step 3
    Failed,
}

/// Which history item is currently receiving deltas, if any. At most one is
/// open at a time; a new `*Start` seals whatever is open first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenKind {
    Message,
}

/// Ordered scrollback of history items plus the wrap-height cache backing the
/// flattened-line scroll model. Heights are per-item `Paragraph::line_count`
/// results, invalidated only when the item's text changes or the width does.
#[derive(Debug, Default)]
pub struct History {
    items: Vec<HistoryItem>,
    open: Option<OpenKind>,
    heights: Vec<Option<usize>>,
    height_width: Option<u16>,
}

impl History {
    pub fn user_prompt(&mut self, text: String) {
        self.push(HistoryItem::UserPrompt(text));
    }

    pub fn message_start(&mut self) {
        self.seal_open();
        self.items.push(HistoryItem::Message(String::new()));
        self.heights.push(None);
        self.open = Some(OpenKind::Message);
    }

    /// Append a chunk to the open message, implicitly starting one if no
    /// message is open (a harness that streams without `MessageStart`).
    pub fn message_delta(&mut self, delta: &str) {
        if self.open != Some(OpenKind::Message) {
            self.message_start();
        }
        if let Some(HistoryItem::Message(text)) = self.items.last_mut() {
            text.push_str(delta);
            if let Some(height) = self.heights.last_mut() {
                *height = None;
            }
        }
    }

    pub fn message_end(&mut self) {
        if self.open == Some(OpenKind::Message) {
            self.open = None;
        }
    }

    /// Seal any open item without ending it as a success — used when the user
    /// interrupts, so a half-streamed item renders coherently whether or not
    /// the harness sends more events.
    pub fn seal_open(&mut self) {
        self.open = None;
    }

    pub fn push(&mut self, item: HistoryItem) {
        self.seal_open();
        self.items.push(item);
        self.heights.push(None);
    }

    pub fn items(&self) -> &[HistoryItem] {
        &self.items
    }

    /// Wrapped height of one item at `width` (in display rows).
    pub fn item_height(&mut self, index: usize, width: u16) -> usize {
        self.ensure_heights(width);
        self.heights[index].unwrap_or(1)
    }

    /// Sum of all item heights at `width`; the extent of the scrollback.
    pub fn total_height(&mut self, width: u16) -> usize {
        self.ensure_heights(width);
        self.heights.iter().map(|height| height.unwrap_or(1)).sum()
    }

    /// Recompute heights lazily: only when the width changes (all items) or an
    /// item's text changed (that item only).
    fn ensure_heights(&mut self, width: u16) {
        if self.height_width != Some(width) {
            for height in &mut self.heights {
                *height = None;
            }
            self.height_width = Some(width);
        }
        for (item, height) in self.items.iter_mut().zip(self.heights.iter_mut()) {
            if height.is_none() {
                *height = Some(item.wrapped_height(width));
            }
        }
    }
}

impl HistoryItem {
    /// Styled display lines for rendering: one `Line` per logical line, with
    /// the theme's per-kind style and code-fence blocks in the dim style,
    /// plus a trailing blank row separating this item from the next.
    pub fn styled_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let base = self.base_style(theme);
        let Some(text) = self.raw_text() else {
            return vec![Line::from("")];
        };
        let mut fenced = false;
        let mut lines = Vec::new();
        for raw in text.split('\n') {
            if raw.trim_start().starts_with("```") {
                fenced = !fenced;
            }
            let style = if fenced { theme.code_fence } else { base };
            lines.push(Line::from(raw.to_string()).style(style));
        }
        lines.push(Line::from("").style(base)); // separator row
        lines
    }

    fn wrapped_height(&self, width: u16) -> usize {
        let Some(text) = self.raw_text() else {
            return 2; // one content row + the separator row
        };
        // The same logical line set as [`Self::styled_lines`] (styles don't
        // affect width), so the trailing separator row is counted and the
        // height stays in sync with what actually renders.
        let mut lines: Vec<Line> = text
            .split('\n')
            .map(|part| Line::from(part.to_string()))
            .collect();
        lines.push(Line::from(""));
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .line_count(width)
    }

    fn base_style(&self, theme: &Theme) -> Style {
        match self {
            HistoryItem::UserPrompt(_) => theme.user_prompt,
            HistoryItem::Thinking(_) => theme.thinking,
            HistoryItem::Message(_) => theme.message,
            HistoryItem::ToolCall(item) => match item.status {
                ToolCallStatus::Running => theme.tool_running,
                ToolCallStatus::Success => theme.tool_success,
                ToolCallStatus::Failed => theme.tool_failure,
            },
            HistoryItem::Error(_) => theme.error,
        }
    }

    fn raw_text(&self) -> Option<&str> {
        match self {
            HistoryItem::UserPrompt(text)
            | HistoryItem::Thinking(text)
            | HistoryItem::Message(text)
            | HistoryItem::Error(text) => Some(text),
            // Tool call rendering (header + full args + tail-4 output) lands
            // in rollout step 3.
            HistoryItem::ToolCall(item) => Some(&item.args),
        }
    }
}
